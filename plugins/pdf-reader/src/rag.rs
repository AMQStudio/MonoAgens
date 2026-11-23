//! Per-page PDF text extraction (`lopdf`) and persistent hybrid BM25 + Cosine RRF RAG engine.

#![forbid(unsafe_code)]

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lopdf::{Document, Object};
use monoagens_body_sdk::sha256_hex;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::PdfReaderConfig;

/// Persistent RAG index filename inside `<state_dir>`.
pub const PDF_RAG_INDEX_FILENAME: &str = "pdf_rag_index.json";

const EMBED_HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const EMBED_BATCH_SIZE: usize = 32;
const PREVIEW_MAX_CHARS: usize = 200;

/// A single page-cited chunk stored in [`PdfRagIndex`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexedChunk {
    pub doc_id: String,
    pub page: u32,
    pub chunk_index: usize,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

/// Complete indexed representation of one PDF document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexedPdfDoc {
    pub doc_id: String,
    pub title: String,
    pub source: String,
    pub page_count: usize,
    pub pages: Vec<String>,
    pub chunks: Vec<IndexedChunk>,
    pub indexed_at_ms: u64,
}

/// Compact summary returned by `pdf.list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub doc_id: String,
    pub title: String,
    pub page_count: usize,
    pub chunk_count: usize,
}

/// Compact metadata returned by `pdf.index` (never dumps full PDF text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexResult {
    pub doc_id: String,
    pub title: String,
    pub page_count: usize,
    pub chunk_count: usize,
    pub indexed_with_embeddings: bool,
    pub preview: String,
}

/// Single page-cited RAG hit returned by `pdf.search`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    pub doc_id: String,
    pub title: String,
    pub page: u32,
    pub chunk_index: usize,
    pub score: f64,
    pub bm25_score: f64,
    pub semantic_score: Option<f64>,
    pub citation: String,
    pub text: String,
}

/// Single bounded page entry returned by `pdf.read_page`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageText {
    pub page: u32,
    pub text: String,
}

/// Bounded page-range response returned by `pdf.read_page`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadPagesResult {
    pub doc_id: String,
    pub start_page: u32,
    pub end_page: u32,
    pub pages: Vec<PageText>,
}

/// Response returned by `pdf.delete`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResult {
    pub doc_id: String,
    pub deleted: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct PersistedRagState {
    documents: BTreeMap<String, IndexedPdfDoc>,
}

/// Persistent page-aware RAG index backed by `<state_dir>/pdf_rag_index.json` when configured.
#[derive(Debug)]
pub struct PdfRagIndex {
    index_file: Option<PathBuf>,
    documents: BTreeMap<String, IndexedPdfDoc>,
}

impl PdfRagIndex {
    /// Loads an existing `pdf_rag_index.json` from `state_dir` if present, or initializes
    /// an empty store (in-memory if `state_dir` is `None`).
    #[must_use]
    pub fn load_or_new(state_dir: Option<&Path>) -> Self {
        let index_file = state_dir.map(|dir| dir.join(PDF_RAG_INDEX_FILENAME));
        let documents = index_file
            .as_deref()
            .and_then(|path| {
                if !path.exists() {
                    return None;
                }
                match fs::read(path) {
                    Ok(raw) => match serde_json::from_slice::<PersistedRagState>(&raw) {
                        Ok(state) => Some(state.documents),
                        Err(err) => {
                            eprintln!(
                                "plugin-pdf-reader: failed to parse '{}': {err}",
                                path.display()
                            );
                            None
                        }
                    },
                    Err(err) => {
                        eprintln!(
                            "plugin-pdf-reader: failed to read '{}': {err}",
                            path.display()
                        );
                        None
                    }
                }
            })
            .unwrap_or_default();

        Self {
            index_file,
            documents,
        }
    }

    /// Returns the path to `pdf_rag_index.json` if file-backed persistence is active.
    #[must_use]
    pub fn index_file(&self) -> Option<&Path> {
        self.index_file.as_deref()
    }

    /// Borrows all indexed documents keyed by `doc_id`.
    #[must_use]
    pub fn documents(&self) -> &BTreeMap<String, IndexedPdfDoc> {
        &self.documents
    }

    /// Parses raw PDF bytes per page, chunks each page with overlap, optionally fetches
    /// document-regime embeddings, persists the index atomically, and returns compact metadata.
    pub fn index_bytes(
        &mut self,
        bytes: &[u8],
        doc_id: Option<&str>,
        title: Option<&str>,
        source: &str,
        indexed_at_ms: u64,
        cfg: &PdfReaderConfig,
    ) -> Result<IndexResult, String> {
        let doc = Document::load_mem(bytes).map_err(|e| format!("failed to parse PDF: {e}"))?;
        let pages = extract_pages_from_document(&doc)?;

        let resolved_doc_id = doc_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| derive_doc_id(bytes, source));

        let resolved_title = title
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .or_else(|| extract_pdf_title(&doc))
            .unwrap_or_else(|| resolved_doc_id.clone());

        let mut chunks = Vec::new();
        for (idx, page_text) in pages.iter().enumerate() {
            let page_num = (idx + 1) as u32;
            let page_chunks = chunk_page_text(
                &resolved_doc_id,
                page_num,
                page_text,
                cfg.chunk_chars,
                cfg.chunk_overlap_chars,
                chunks.len(),
            );
            chunks.extend(page_chunks);
        }

        let mut indexed_with_embeddings = false;
        if let Some(endpoint) = cfg.resolved_embed_endpoint()
            && !chunks.is_empty()
        {
            let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();
            match fetch_embeddings_batched(&endpoint, &texts, "document") {
                Ok(embeddings) if embeddings.len() == chunks.len() => {
                    for (chunk, emb) in chunks.iter_mut().zip(embeddings) {
                        chunk.embedding = Some(emb);
                    }
                    indexed_with_embeddings = true;
                }
                Ok(embeddings) => {
                    eprintln!(
                        "plugin-pdf-reader: embedding count mismatch (expected {}, got {}), falling back to BM25",
                        chunks.len(),
                        embeddings.len()
                    );
                }
                Err(err) => {
                    eprintln!(
                        "plugin-pdf-reader: embedding sidecar unavailable during index ({err}), falling back to BM25"
                    );
                }
            }
        }

        let preview = build_preview(&pages);
        let page_count = pages.len();
        let chunk_count = chunks.len();

        let record = IndexedPdfDoc {
            doc_id: resolved_doc_id.clone(),
            title: resolved_title.clone(),
            source: source.to_owned(),
            page_count,
            pages,
            chunks,
            indexed_at_ms,
        };

        self.documents.insert(resolved_doc_id.clone(), record);
        self.save()?;

        Ok(IndexResult {
            doc_id: resolved_doc_id,
            title: resolved_title,
            page_count,
            chunk_count,
            indexed_with_embeddings,
            preview,
        })
    }

    /// Searches indexed chunks via BM25 and (when configured and available) hybrid cosine + RRF.
    pub fn search(
        &self,
        query: &str,
        doc_id_filter: Option<&str>,
        top_k: Option<usize>,
        cfg: &PdfReaderConfig,
    ) -> Result<Vec<SearchHit>, String> {
        let trimmed_query = query.trim();
        if trimmed_query.is_empty() {
            return Err("search query must not be empty".to_owned());
        }

        let doc_filter = doc_id_filter.map(str::trim).filter(|s| !s.is_empty());
        if let Some(filter_id) = doc_filter
            && !self.documents.contains_key(filter_id)
        {
            return Err(format!("document '{filter_id}' is not indexed"));
        }

        let k = top_k
            .unwrap_or(cfg.default_top_k)
            .clamp(1, cfg.max_top_k.max(1));

        let candidates: Vec<(&IndexedPdfDoc, &IndexedChunk)> = self
            .documents
            .values()
            .filter(|doc| doc_filter.is_none_or(|id| doc.doc_id == id))
            .flat_map(|doc| doc.chunks.iter().map(move |chunk| (doc, chunk)))
            .collect();

        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let bm25_scores = compute_bm25_scores(&candidates, trimmed_query, cfg.bm25_k1, cfg.bm25_b);

        let any_chunk_embeddings = candidates.iter().any(|(_, c)| c.embedding.is_some());
        let query_embedding = if any_chunk_embeddings
            && let Some(endpoint) = cfg.resolved_embed_endpoint()
        {
            match fetch_embeddings_batched(&endpoint, &[trimmed_query], "query") {
                Ok(mut vecs) if !vecs.is_empty() => Some(vecs.remove(0)),
                Ok(_) => None,
                Err(err) => {
                    eprintln!(
                        "plugin-pdf-reader: embedding sidecar unavailable during search ({err}), falling back to BM25"
                    );
                    None
                }
            }
        } else {
            None
        };

        if let Some(q_emb) = query_embedding {
            let semantic_scores: Vec<Option<f64>> = candidates
                .iter()
                .map(|(_, chunk)| {
                    chunk
                        .embedding
                        .as_deref()
                        .and_then(|c_emb| cosine_similarity(&q_emb, c_emb))
                })
                .collect();

            if semantic_scores.iter().any(Option::is_some) {
                return Ok(fuse_rrf_hits(
                    &candidates,
                    &bm25_scores,
                    &semantic_scores,
                    cfg.rrf_k,
                    k,
                ));
            }
        }

        // BM25-only ranking (exclude zero-score non-matching chunks).
        let mut ranked: Vec<(usize, f64)> = bm25_scores
            .iter()
            .copied()
            .enumerate()
            .filter(|&(_, score)| score > 0.0)
            .collect();

        ranked.sort_by(|&(idx_a, score_a), &(idx_b, score_b)| {
            cmp_f64_desc(score_a, score_b).then_with(|| cmp_candidate(&candidates[idx_a], &candidates[idx_b]))
        });
        ranked.truncate(k);

        Ok(ranked
            .into_iter()
            .map(|(idx, bm25)| build_search_hit(&candidates[idx], bm25, bm25, None))
            .collect())
    }

    /// Reads a bounded range of pages `[start_page, end_page]` (capped to `max_pages_per_read`).
    pub fn read_page(
        &self,
        doc_id: &str,
        start_page: u32,
        end_page: Option<u32>,
        max_pages_per_read: usize,
    ) -> Result<ReadPagesResult, String> {
        let doc = self
            .documents
            .get(doc_id)
            .ok_or_else(|| format!("document '{doc_id}' is not indexed"))?;

        if doc.page_count == 0 {
            return Err(format!("document '{doc_id}' has 0 pages"));
        }
        let total_pages = doc.page_count as u32;
        if start_page == 0 || start_page > total_pages {
            return Err(format!(
                "start_page {start_page} out of range (1..={total_pages}) for document '{doc_id}'"
            ));
        }

        let requested_end = end_page.unwrap_or(start_page);
        if requested_end < start_page {
            return Err(format!(
                "end_page ({requested_end}) must be >= start_page ({start_page})"
            ));
        }

        let max_span = max_pages_per_read.max(1) as u32;
        let capped_end = start_page
            .saturating_add(max_span.saturating_sub(1))
            .min(requested_end)
            .min(total_pages);

        let mut pages = Vec::with_capacity((capped_end - start_page + 1) as usize);
        for page_num in start_page..=capped_end {
            let text = doc
                .pages
                .get((page_num - 1) as usize)
                .cloned()
                .unwrap_or_default();
            pages.push(PageText {
                page: page_num,
                text,
            });
        }

        Ok(ReadPagesResult {
            doc_id: doc.doc_id.clone(),
            start_page,
            end_page: capped_end,
            pages,
        })
    }

    /// Lists compact summaries for all indexed documents.
    #[must_use]
    pub fn list(&self) -> Vec<DocumentSummary> {
        self.documents
            .values()
            .map(|doc| DocumentSummary {
                doc_id: doc.doc_id.clone(),
                title: doc.title.clone(),
                page_count: doc.page_count,
                chunk_count: doc.chunks.len(),
            })
            .collect()
    }

    /// Removes a document and its chunks from the index and persists atomically.
    pub fn delete(&mut self, doc_id: &str) -> Result<DeleteResult, String> {
        let deleted = self.documents.remove(doc_id).is_some();
        if deleted {
            self.save()?;
        }
        Ok(DeleteResult {
            doc_id: doc_id.to_owned(),
            deleted,
        })
    }

    /// Atomically persists the index to `<state_dir>/pdf_rag_index.json` when configured.
    pub fn save(&self) -> Result<(), String> {
        let Some(path) = &self.index_file else {
            return Ok(());
        };
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "failed to create RAG index directory '{}': {e}",
                    parent.display()
                )
            })?;
        }
        let state = PersistedRagState {
            documents: self.documents.clone(),
        };
        let encoded = serde_json::to_vec(&state)
            .map_err(|e| format!("failed to serialize PDF RAG index: {e}"))?;
        let tmp = path.with_extension("tmp");
        fs::write(&tmp, encoded).map_err(|e| {
            format!(
                "failed to write temporary RAG index '{}': {e}",
                tmp.display()
            )
        })?;
        fs::rename(&tmp, path).map_err(|e| {
            format!(
                "failed to atomically replace RAG index '{}': {e}",
                path.display()
            )
        })?;
        Ok(())
    }
}

/// Extracts normalized text per page (`1..=N`) from raw PDF bytes using `lopdf`.
pub fn extract_pdf_pages(bytes: &[u8]) -> Result<Vec<String>, String> {
    let doc = Document::load_mem(bytes).map_err(|e| format!("failed to parse PDF: {e}"))?;
    extract_pages_from_document(&doc)
}

fn extract_pages_from_document(doc: &Document) -> Result<Vec<String>, String> {
    let pages_map = doc.get_pages();
    if pages_map.is_empty() {
        return Err("PDF contains no pages".to_owned());
    }

    let mut pages = Vec::with_capacity(pages_map.len());
    for &page_num in pages_map.keys() {
        let raw = doc
            .extract_text(&[page_num])
            .map_err(|e| format!("failed to extract text from page {page_num}: {e}"))?;
        pages.push(normalize_whitespace(&raw));
    }
    Ok(pages)
}

fn extract_pdf_title(doc: &Document) -> Option<String> {
    let info_obj = doc.trailer.get(b"Info").ok()?;
    let info_dict = match info_obj {
        Object::Reference(id) => doc.get_dictionary(*id).ok()?,
        Object::Dictionary(dict) => dict,
        _ => return None,
    };
    let title_obj = info_dict.get(b"Title").ok()?;
    let bytes = match title_obj {
        Object::String(b, _) => b.as_slice(),
        _ => return None,
    };
    let decoded = String::from_utf8_lossy(bytes).trim().to_owned();
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

/// Normalizes extracted PDF text by trimming lines and collapsing redundant whitespace.
#[must_use]
pub fn normalize_whitespace(raw: &str) -> String {
    let normalized_newlines = raw.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<String> = normalized_newlines
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty())
        .collect();
    lines.join("\n")
}

/// Splits a single page's text into UTF-8 safe overlapping chunks respecting word boundaries.
#[must_use]
pub fn chunk_page_text(
    doc_id: &str,
    page: u32,
    text: &str,
    chunk_chars: usize,
    chunk_overlap_chars: usize,
    starting_chunk_index: usize,
) -> Vec<IndexedChunk> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let max_chars = chunk_chars.max(1);
    let overlap = chunk_overlap_chars.min(max_chars.saturating_sub(1));

    let char_bounds: Vec<(usize, char)> = trimmed.char_indices().collect();
    let total_chars = char_bounds.len();

    let byte_at = |char_idx: usize| -> usize {
        if char_idx >= total_chars {
            trimmed.len()
        } else {
            char_bounds[char_idx].0
        }
    };

    let mut chunks = Vec::new();
    let mut start = 0usize;

    while start < total_chars {
        let hard_end = (start + max_chars).min(total_chars);
        let mut end = hard_end;

        // Snap end back to a whitespace boundary when not at the end of the page.
        if hard_end < total_chars {
            let min_snap = start + (max_chars / 2).max(1);
            if let Some(ws_idx) = (min_snap..=hard_end)
                .rev()
                .find(|&idx| idx < total_chars && char_bounds[idx].1.is_whitespace())
            {
                end = ws_idx;
            }
        }

        let slice = trimmed[byte_at(start)..byte_at(end)].trim();
        if !slice.is_empty() {
            chunks.push(IndexedChunk {
                doc_id: doc_id.to_owned(),
                page,
                chunk_index: starting_chunk_index + chunks.len(),
                text: slice.to_owned(),
                embedding: None,
            });
        }

        if end >= total_chars {
            break;
        }

        // Advance start with overlap, nudging to a clean word start if helpful,
        // while strictly advancing past `start`.
        let mut next_start = end.saturating_sub(overlap);
        if next_start <= start {
            next_start = end;
        } else if next_start > 0
            && !char_bounds[next_start - 1].1.is_whitespace()
            && let Some(ws_idx) = (next_start..end).find(|&idx| char_bounds[idx].1.is_whitespace())
            && ws_idx + 1 < end
        {
            next_start = ws_idx + 1;
        }

        while next_start < total_chars && char_bounds[next_start].1.is_whitespace() {
            next_start += 1;
        }
        if next_start <= start {
            next_start = (start + 1).min(total_chars);
        }
        start = next_start;
    }

    chunks
}

fn derive_doc_id(bytes: &[u8], source: &str) -> String {
    if !source.is_empty()
        && source != "bytes_base64"
        && let Some(stem) = Path::new(source).file_stem().and_then(|s| s.to_str())
    {
        let clean: String = stem
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .collect();
        let trimmed = clean.trim_matches('-');
        if !trimmed.is_empty() {
            return trimmed.to_owned();
        }
    }
    let digest = sha256_hex(bytes);
    format!("pdf-{}", &digest[..12])
}

fn build_preview(pages: &[String]) -> String {
    let first_non_empty = pages
        .iter()
        .find(|p| !p.trim().is_empty())
        .map(String::as_str)
        .unwrap_or("");
    let compact = first_non_empty
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let mut chars = compact.chars();
    let preview: String = chars.by_ref().take(PREVIEW_MAX_CHARS).collect();
    if chars.next().is_some() {
        format!("{preview}...")
    } else {
        preview
    }
}

/// Tokenizes text into lowercase alphanumeric terms for BM25 lexical scoring.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_alphanumeric() {
            for lc in ch.to_lowercase() {
                current.push(lc);
            }
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

fn compute_bm25_scores(
    candidates: &[(&IndexedPdfDoc, &IndexedChunk)],
    query: &str,
    k1: f64,
    b: f64,
) -> Vec<f64> {
    let query_terms = tokenize(query);
    if query_terms.is_empty() || candidates.is_empty() {
        return vec![0.0; candidates.len()];
    }

    let mut doc_term_freqs: Vec<HashMap<String, usize>> = Vec::with_capacity(candidates.len());
    let mut doc_lengths: Vec<usize> = Vec::with_capacity(candidates.len());
    let mut df: HashMap<String, usize> = HashMap::new();
    let unique_query_terms: HashSet<&str> = query_terms.iter().map(String::as_str).collect();

    for &(_, chunk) in candidates {
        let tokens = tokenize(&chunk.text);
        doc_lengths.push(tokens.len());
        let mut tf_map: HashMap<String, usize> = HashMap::new();
        for tok in tokens {
            *tf_map.entry(tok).or_insert(0) += 1;
        }
        for &qt in &unique_query_terms {
            if tf_map.contains_key(qt) {
                *df.entry(qt.to_owned()).or_insert(0) += 1;
            }
        }
        doc_term_freqs.push(tf_map);
    }

    let n = candidates.len() as f64;
    let total_len: usize = doc_lengths.iter().sum();
    let avgdl = if total_len == 0 {
        1.0
    } else {
        total_len as f64 / n
    };

    let mut scores = Vec::with_capacity(candidates.len());
    for (idx, tf_map) in doc_term_freqs.iter().enumerate() {
        let dl = doc_lengths[idx] as f64;
        let mut score = 0.0;
        for qt in &query_terms {
            let tf = *tf_map.get(qt.as_str()).unwrap_or(&0) as f64;
            if tf == 0.0 {
                continue;
            }
            let term_df = *df.get(qt.as_str()).unwrap_or(&0) as f64;
            let idf = ((n - term_df + 0.5) / (term_df + 0.5) + 1.0).ln();
            let norm = tf + k1 * (1.0 - b + b * (dl / avgdl));
            let tf_weight = if norm > 0.0 {
                (tf * (k1 + 1.0)) / norm
            } else {
                0.0
            };
            score += idf * tf_weight;
        }
        scores.push(score);
    }
    scores
}

fn fuse_rrf_hits(
    candidates: &[(&IndexedPdfDoc, &IndexedChunk)],
    bm25_scores: &[f64],
    semantic_scores: &[Option<f64>],
    rrf_k: f64,
    top_k: usize,
) -> Vec<SearchHit> {
    let mut bm25_order: Vec<usize> = bm25_scores
        .iter()
        .enumerate()
        .filter(|&(_, &s)| s > 0.0)
        .map(|(i, _)| i)
        .collect();
    bm25_order.sort_by(|&a, &b| {
        cmp_f64_desc(bm25_scores[a], bm25_scores[b])
            .then_with(|| cmp_candidate(&candidates[a], &candidates[b]))
    });

    let mut sem_order: Vec<usize> = semantic_scores
        .iter()
        .enumerate()
        .filter_map(|(i, s)| s.map(|_| i))
        .collect();
    sem_order.sort_by(|&a, &b| {
        cmp_f64_desc(
            semantic_scores[a].unwrap_or(f64::NEG_INFINITY),
            semantic_scores[b].unwrap_or(f64::NEG_INFINITY),
        )
        .then_with(|| cmp_candidate(&candidates[a], &candidates[b]))
    });

    let mut rrf_scores = vec![0.0_f64; candidates.len()];
    let mut included = vec![false; candidates.len()];

    for (rank_zero, &idx) in bm25_order.iter().enumerate() {
        let rank = (rank_zero + 1) as f64;
        rrf_scores[idx] += 1.0 / (rrf_k + rank);
        included[idx] = true;
    }
    for (rank_zero, &idx) in sem_order.iter().enumerate() {
        let rank = (rank_zero + 1) as f64;
        rrf_scores[idx] += 1.0 / (rrf_k + rank);
        included[idx] = true;
    }

    let mut fused: Vec<usize> = (0..candidates.len()).filter(|&i| included[i]).collect();
    fused.sort_by(|&a, &b| {
        cmp_f64_desc(rrf_scores[a], rrf_scores[b])
            .then_with(|| cmp_candidate(&candidates[a], &candidates[b]))
    });
    fused.truncate(top_k);

    fused
        .into_iter()
        .map(|idx| {
            build_search_hit(
                &candidates[idx],
                rrf_scores[idx],
                bm25_scores[idx],
                semantic_scores[idx],
            )
        })
        .collect()
}

fn build_search_hit(
    candidate: &(&IndexedPdfDoc, &IndexedChunk),
    score: f64,
    bm25_score: f64,
    semantic_score: Option<f64>,
) -> SearchHit {
    let (doc, chunk) = candidate;
    SearchHit {
        doc_id: chunk.doc_id.clone(),
        title: doc.title.clone(),
        page: chunk.page,
        chunk_index: chunk.chunk_index,
        score,
        bm25_score,
        semantic_score,
        citation: format!("[{} p.{}]", chunk.doc_id, chunk.page),
        text: chunk.text.clone(),
    }
}

fn cmp_f64_desc(a: f64, b: f64) -> Ordering {
    b.partial_cmp(&a).unwrap_or(Ordering::Equal)
}

fn cmp_candidate(a: &(&IndexedPdfDoc, &IndexedChunk), b: &(&IndexedPdfDoc, &IndexedChunk)) -> Ordering {
    a.1.doc_id
        .cmp(&b.1.doc_id)
        .then_with(|| a.1.page.cmp(&b.1.page))
        .then_with(|| a.1.chunk_index.cmp(&b.1.chunk_index))
}

/// Computes cosine similarity between two equal-length vectors.
#[must_use]
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.is_empty() || a.len() != b.len() {
        return None;
    }
    let mut dot = 0.0_f64;
    let mut norm_a = 0.0_f64;
    let mut norm_b = 0.0_f64;
    for (&x, &y) in a.iter().zip(b.iter()) {
        let xf = f64::from(x);
        let yf = f64::from(y);
        dot += xf * yf;
        norm_a += xf * xf;
        norm_b += yf * yf;
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 || !denom.is_finite() {
        None
    } else {
        Some(dot / denom)
    }
}

fn normalize_embed_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.ends_with("/embed") {
        trimmed.to_owned()
    } else {
        format!("{trimmed}/embed")
    }
}

fn fetch_embeddings_batched(
    endpoint: &str,
    texts: &[&str],
    regime: &str,
) -> Result<Vec<Vec<f32>>, String> {
    let client = Client::builder()
        .timeout(EMBED_HTTP_TIMEOUT)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;

    let url = normalize_embed_url(endpoint);
    let mut all_embeddings = Vec::with_capacity(texts.len());

    for batch in texts.chunks(EMBED_BATCH_SIZE) {
        let payload = json!({
            "texts": batch,
            "regime": regime,
        });
        let resp = client
            .post(&url)
            .json(&payload)
            .send()
            .map_err(|e| format!("POST {url} failed: {e}"))?;
        let status = resp.status();
        let body = resp
            .text()
            .map_err(|e| format!("failed to read embed response body: {e}"))?;
        if !status.is_success() {
            return Err(format!("embed endpoint returned HTTP {status}: {body}"));
        }
        let parsed = decode_embed_response(&body, batch.len())?;
        all_embeddings.extend(parsed);
    }

    Ok(all_embeddings)
}

fn decode_embed_response(raw: &str, expected_count: usize) -> Result<Vec<Vec<f32>>, String> {
    let val: Value =
        serde_json::from_str(raw).map_err(|e| format!("invalid embed JSON response: {e}"))?;

    if let Some(arr) = val.get("embeddings").and_then(Value::as_array) {
        let vecs = arr
            .iter()
            .map(parse_f32_vec)
            .collect::<Result<Vec<_>, _>>()?;
        if vecs.len() != expected_count {
            return Err(format!(
                "expected {expected_count} embeddings, got {}",
                vecs.len()
            ));
        }
        return Ok(vecs);
    }

    if expected_count == 1
        && let Some(single) = val.get("embedding")
    {
        return Ok(vec![parse_f32_vec(single)?]);
    }

    if let Some(data) = val.get("data").and_then(Value::as_array) {
        let mut vecs = Vec::with_capacity(data.len());
        for item in data {
            let emb = item
                .get("embedding")
                .ok_or_else(|| "embed response data item missing 'embedding'".to_owned())?;
            vecs.push(parse_f32_vec(emb)?);
        }
        if vecs.len() != expected_count {
            return Err(format!(
                "expected {expected_count} embeddings in data[], got {}",
                vecs.len()
            ));
        }
        return Ok(vecs);
    }

    Err("embed JSON response missing 'embeddings', 'embedding', or 'data'".to_owned())
}

fn parse_f32_vec(val: &Value) -> Result<Vec<f32>, String> {
    let arr = val
        .as_array()
        .ok_or_else(|| "embedding vector is not a JSON array".to_owned())?;
    let mut out = Vec::with_capacity(arr.len());
    for item in arr {
        let f = item
            .as_f64()
            .ok_or_else(|| "embedding element is not a number".to_owned())?;
        out.push(f as f32);
    }
    if out.is_empty() {
        return Err("embedding vector must not be empty".to_owned());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    /// Builds a real multi-page PDF document in memory via `lopdf`.
    pub(crate) fn build_test_pdf(page_texts: &[&str]) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });

        let mut kids: Vec<Object> = Vec::with_capacity(page_texts.len());
        for text in page_texts {
            let escaped = text
                .replace('\\', "\\\\")
                .replace('(', "\\(")
                .replace(')', "\\)");
            let stream_bytes = format!("BT /F1 12 Tf 72 720 Td ({escaped}) Tj ET").into_bytes();
            let content_id = doc.add_object(lopdf::Stream::new(dictionary! {}, stream_bytes));
            let page_id = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            });
            kids.push(page_id.into());
        }

        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => kids,
                "Count" => page_texts.len() as i64,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);

        let mut out = Vec::new();
        doc.save_to(&mut out).expect("save test PDF");
        out
    }

    #[test]
    fn chunking_is_utf8_safe_and_respects_overlap() {
        let text = "Rust ownership ensures memory safety without garbage collection. \
                    Borrow checking prevents data races at compile time across threads. \
                    Lifetimes verify references never outlive their underlying data.";
        let chunks = chunk_page_text("doc1", 2, text, 80, 20, 0);
        assert!(chunks.len() >= 2);
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.doc_id, "doc1");
            assert_eq!(c.page, 2);
            assert_eq!(c.chunk_index, i);
            assert!(c.text.chars().count() <= 80);
        }

        // Multibyte UTF-8 text must never panic on char boundaries.
        let utf8_text = "Kopi tubruk Indonesia sangat nikmat — café résumé naïve — 日本語テスト。 ".repeat(4);
        let utf8_chunks = chunk_page_text("doc-utf8", 1, &utf8_text, 40, 10, 0);
        assert!(utf8_chunks.len() >= 2);
    }

    #[test]
    fn extract_and_bm25_search_on_real_lopdf_pdf() {
        let pdf_bytes = build_test_pdf(&[
            "Rust memory safety relies on ownership borrowing and zero cost abstractions.",
            "WhatsApp encryption uses the Signal protocol with end to end key agreement.",
            "Indonesian coffee recipes include kopi tubruk and Aceh Gayo pour over brewing.",
        ]);

        let pages = extract_pdf_pages(&pdf_bytes).unwrap();
        assert_eq!(pages.len(), 3);
        assert!(pages[1].contains("WhatsApp encryption"));

        let mut index = PdfRagIndex::load_or_new(None);
        let cfg = PdfReaderConfig::default();
        let res = index
            .index_bytes(&pdf_bytes, Some("handbook"), Some("Handbook"), "test.pdf", 100, &cfg)
            .unwrap();
        assert_eq!(res.page_count, 3);
        assert_eq!(res.chunk_count, 3);
        assert!(!res.indexed_with_embeddings);

        let hits = index.search("WhatsApp encryption", None, Some(3), &cfg).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].page, 2);
        assert_eq!(hits[0].citation, "[handbook p.2]");
    }
}
