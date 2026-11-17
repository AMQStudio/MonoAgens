# Run 4 Reference Note — R-56 & R-57 Framework Configuration & Ingestion Granularity

Verified against installed packages in `lab/adapters/.venv` and upstream repos on `2026-11-18`:
- `mem0ai==2.2.1` (`mem0ai/mem0` @ `f1791daa86d469249c9b002f62644d58a1eb96a0`)
- `letta-client==1.12.1` (`letta-ai/letta-python`, `docs.letta.com`)
- `graphiti-core==0.31.2` (`getzep/graphiti` @ `4ef979c74e1e68d08217cf0af4022ddbc89c257f`)
- `langmem==0.0.30` + `langgraph==1.2.22` (`langchain-ai/langmem` @ `4beb9c36a40953f09357e5b2dd62ecc726f22d13`)

---

## 1. Mem0 (`mem0ai==2.2.1`)

- **LLM configuration (`mem0/llms/openai.py`, `mem0/configs/llms/openai.py`):**
  - `Memory.from_config({"llm": {"provider": "openai", "config": {...}}})` accepts:
    - `openai_base_url`: URL of the Lab's local metering proxy (e.g., `http://127.0.0.1:<port>/v1`). Note: **unset `OPENROUTER_API_KEY`**, because `OpenAILLM.__init__` checks `os.environ.get("OPENROUTER_API_KEY")` before `openai_base_url`!
    - `api_key`: placeholder or proxy key.
    - `model`: `"deepseek-v4.1-flash"`.
    - `reasoning_effort`: `"high"` + **`is_reasoning_model`: `True`** (otherwise `_is_reasoning_model("deepseek-v4.1-flash")` returns `False` and drops `reasoning_effort`). Note that when `is_reasoning_model=True`, Mem0 drops `temperature` and `max_tokens`.
  - **Custom headers gap:** `OpenAILLM.__init__` constructs `OpenAI(api_key=api_key, base_url=base_url)` **without** `default_headers` or `http_client`. Therefore the Lab's local metering proxy is **required** to inject `x-opencode-session`, `User-Agent`, and `reasoning_effort="high"`.
- **Embedder configuration (`mem0/embeddings/openai.py`, `mem0/configs/vector_stores/qdrant.py`):**
  - `embedder`: `{"provider": "openai", "config": {"model": "embeddinggemma-2", "openai_base_url": "http://127.0.0.1:<embed_port>/v1", "api_key": "local", "embedding_dims": 256}}`.
  - `vector_store`: `{"provider": "qdrant", "config": {"collection_name": "mem0_eval", "path": "<tmp_dir>/qdrant", "embedding_model_dims": 256, "on_disk": False}}`.
  - Setting `embedding_dims=256` makes `OpenAIEmbedding.embed()` send `"dimensions": 256` in `/v1/embeddings`; `Memory.entity_store` lazily clones `vector_store.config` (`mem0/memory/main.py`), so both collections use 256d.
- **Infrastructure & RAM:** Zero external servers/Docker needed (`qdrant-client>=1.12.0` + `sqlalchemy` SQLite are base dependencies). Local Qdrant (`path=...`) + SQLite (`history_db_path`) uses ~100–250 MiB RSS.
- **Public API & Isolation:**
  - Ingest: `memory.add(messages, user_id=conv_scope, metadata={"created_at": iso_ts}, infer=True)` (in `mem0ai 2.2.1`, v3 phased batch pipeline executes **1 LLM extraction/consolidation call** + 1 search embed + 1 batch add embed per `add()`).
  - Retrieve: `memory.search(query, filters={"user_id": conv_scope}, top_k=...)["results"]` -> render into the Lab's shared answer template.
  - Isolate/Reset: unique `user_id` per conversation (or per speaker-conversation pair if replicating `arXiv:2504.19413`), cleared via `memory.delete_all(user_id=...)` or `memory.reset()`.

---

## 2. Letta (`letta-client==1.12.1` + Server)

- **Architecture split in Letta docs (`2026-11-18`):**
  - `letta-client 1.12.1` is a REST client for the **V1 Letta server** (`http://localhost:8283`, Docker `letta/letta:latest` with embedded PostgreSQL + `pgvector`).
  - Letta's current docs (`https://docs.letta.com/v1-sdk/docker`, `https://docs.letta.com/self-hosting`) mark the Docker server as **legacy/deprecated** and promote the **Letta App Server** (`npm install -g @letta-ai/letta-code`, `letta server --backend local`, Node.js 22.19+, TypeScript `@letta-ai/letta-agent-sdk`), which does **not** use `letta-client`.
- **LLM & Header Routing:**
  - In V1 Docker server (`letta/letta:latest`), OpenAI-compatible base URL is configured via `OPENAI_BASE_URL` / `OLLAMA_BASE_URL` env vars on the container (pointing to the Lab's metering proxy on the host via `--network host` on Linux, or `host.docker.internal`) or via `llm_config` (`model_endpoint_type="openai"`, `model_endpoint="http://127.0.0.1:<proxy_port>/v1"`, `reasoning_effort="high"`).
  - Neither `letta-client` nor Letta App Server provides native per-conversation `x-opencode-session` injection to upstream OpenAI calls; the proxy must attach `x-opencode-session` per active conversation (requiring sequential conversation execution per Letta server instance) and inject `reasoning_effort="high"` + `User-Agent`.
- **Embedder & Dimensions:**
  - V1 `EmbeddingConfig(embedding_endpoint_type="openai", embedding_endpoint="http://127.0.0.1:<embed_port>/v1", embedding_model="embeddinggemma-2", embedding_dim=256)`.
  - **Caveat to verify on container startup:** Legacy V1 Letta `pgvector` tables often fix `Vector(4096)` (padding shorter vectors) or `Vector(1536)`; if `letta/letta:latest` rejects `embedding_dim=256`, the sidecar or adapter would need zero-padding to 1536 (recorded as a parity deviation) or use Letta's default embedder (gap/deviation).
- **Infrastructure & RAM:** Legacy Docker `letta/letta:latest` bundles FastAPI + Postgres/pgvector (~1–2.5 GiB RSS). Fits on the 14 GiB laptop (`~5.0 GiB` currently available) if run sequentially (no Neo4j/other heavy Docker containers running concurrently).
- **Public API & Isolation (`letta-ai/letta-leaderboard` LoCoMo pattern):**
  - FileSystem ingestion (published LoCoMo protocol, **0 LLM calls during ingestion**, optional `+1` summary call if enabled): `source = client.sources.create(...)`, `client.sources.files.upload(source_id=..., file=...)` per session file, `agent = client.agents.create(tools=["search_files", "answer_question"], ...)`, `client.agents.sources.attach(agent.id, source_id)`.
  - Answer: `client.agents.messages.create(agent_id=agent.id, messages=[{"role": "user", "content": question}])`.
  - Isolate/Reset: one `agent.id` + `source.id` per conversation, deleted via `client.agents.delete(agent.id)` / `client.sources.delete(source.id)`.

---

## 3. Zep / Graphiti (`graphiti-core==0.31.2`)

- **LLM configuration (`graphiti_core/llm_client/openai_generic_client.py`):**
  - Use `OpenAIGenericClient` (Chat Completions API — required because `OpenAIClient` uses OpenAI's Responses API `responses.parse`, whereas OpenCode Go serves `deepseek-v4.1-flash` on `/openai/v1/chat/completions`!) with `structured_output_mode='json_object'` (specifically documented in `graphiti-core 0.31.2` for DeepSeek!).
  - Pass an `AsyncOpenAI(api_key=..., base_url="http://127.0.0.1:<proxy_port>/v1", default_headers={"x-opencode-session": ..., "User-Agent": ...})` directly via `client=...`.
  - Note: `OpenAIGenericClient._generate_response` does **not** send `reasoning_effort`, so the Lab's metering proxy must inject `reasoning_effort="high"`.
- **Embedder configuration (`graphiti_core/embedder/openai.py`):**
  - `OpenAIEmbedder(config=OpenAIEmbedderConfig(embedding_model="embeddinggemma-2", embedding_dim=256, base_url="http://127.0.0.1:<embed_port>/v1", api_key="local"))`.
  - `OpenAIEmbedder.create` does **not** send `dimensions` in the request; it slices `result.data[0].embedding[:self.config.embedding_dim]` client-side. Graph drivers (`Neo4jDriver`, `FalkorDriver`, `KuzuDriver`) store variable-length float arrays, so `256` works cleanly on a fresh DB.
- **Infrastructure & RAM:**
  - `graphiti-core==0.31.2` includes `neo4j>=5.26.0` by default; `falkordb`, `falkordblite`, and `kuzu` are optional extras (not currently in `lab/adapters/pyproject.toml`).
  - **FalkorDB Docker** (`falkordb/falkordb:latest`, ~0.5–1.5 GiB RSS) or **`graphiti-core[falkordblite]`** (embedded Python 3.12+, zero Docker) is much lighter on a 14 GiB RAM laptop than **Neo4j Docker** (JVM, ~1.5–3 GiB RSS). `KuzuDriver` is in-process (<200 MiB) but marked **deprecated** in `graphiti-core` README.
- **Public API, Isolation & Ingestion Cost:**
  - Ingest: `await graphiti.add_episode(name=..., episode_body=..., source_description=..., reference_time=dt, source=EpisodeType.message, group_id=conv_id)`. Each episode costs **2–6+ LLM calls** (`extract_nodes`, optional `resolve_extracted_nodes`, `extract_edges`, optional `resolve_extracted_edges`, `extract_attributes_from_nodes` attribute + summary batch).
  - Retrieve: `await graphiti.search_(query, config=COMBINED_HYBRID_SEARCH_CROSS_ENCODER, group_ids=[conv_id])` (or `graphiti.search(...)`), format `<FACTS>` + `<ENTITIES>`, pass to shared answer template.
  - Isolate/Reset: `group_id=conv_id` and `clear_data(graphiti.driver, group_ids=[conv_id])`.

---

## 4. LangMem (`langmem==0.0.30` + `langgraph==1.2.22`)

- **LLM configuration (`langchain_openai.ChatOpenAI`):**
  - Instantiate `llm = ChatOpenAI(model="deepseek-v4.1-flash", openai_api_base="http://127.0.0.1:<proxy_port>/v1", openai_api_key="proxy", default_headers={"x-opencode-session": ..., "User-Agent": ...}, reasoning_effort="high")` and pass it to `create_memory_store_manager(llm, ...)` / `create_memory_manager(llm, ...)` and the answerer. Still route through the proxy for unified metering.
- **Embedder configuration (`langgraph.store.memory.InMemoryStore`):**
  - `InMemoryStore(index={"dims": 256, "embed": embed_fn})` where `embed_fn(texts: list[str]) -> list[list[float]]` calls the local `/v1/embeddings` endpoint (accepts any `dims`, including `256`).
- **Infrastructure & RAM:** Zero external servers/Docker; `InMemoryStore` runs in-process (~100–200 MiB RSS). Fits trivially on 14 GiB RAM.
- **Public API, Isolation & Ingestion Cost:**
  - Ingest (`MemoryStoreManager`): `await manager.ainvoke({"messages": unit}, config={"configurable": {"langgraph_user_id": conv_id}})`. With `max_steps=1` and `query_model=None`, costs **1 LLM call** (via `trustcall` extractor) per invoked unit (message, pair, or session). By contrast, Mem0's LoCoMo paper (`evaluation/src/langmem.py`) ran `create_react_agent` per single message (**1–2+ LLM calls per message** × 2 speaker agents).
  - Retrieve & Answer: `results = await store.asearch(("memories", conv_id), query=question, limit=...)` -> pass to shared answer template.
  - Isolate/Reset: namespace tuple `("memories", conv_id)`, cleared by instantiating a fresh `InMemoryStore()` on `reset`.
