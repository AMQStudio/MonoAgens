use std::fs;
use std::io::Write;
use std::path::Path;

use monoagens_core::{EventRecord, Store};
use monoagens_gateway::governance::{JobCtl, export_learning};
use serde_json::json;

use crate::ExportLearningArgs;
use crate::common::{load_paths, open_store_readonly, write_json_pretty};

pub(crate) fn cmd_export_learning(
    config_path: Option<&Path>,
    json_mode: bool,
    args: ExportLearningArgs,
    out: &mut dyn Write,
) -> Result<u8, String> {
    let (_cfg_opt, paths) = load_paths(config_path)?;
    let events: Vec<EventRecord> = if let Some(log_path) = &args.log_json {
        let raw = fs::read_to_string(log_path)
            .map_err(|e| format!("failed to read log JSON '{}': {e}", log_path.display()))?;
        serde_json::from_str(&raw)
            .map_err(|e| format!("failed to parse log JSON '{}': {e}", log_path.display()))?
    } else {
        let data_dir = args.data_dir.clone().unwrap_or(paths.data_dir);
        let store = open_store_readonly(&data_dir)?;
        store
            .read_in_order(None, usize::MAX)
            .map_err(|e| format!("failed to read store log: {e}"))?
    };

    let exp = export_learning(
        &events,
        args.include_private,
        args.place.as_deref(),
        &JobCtl::none(),
    )?;

    if let Some(parent) = args.out.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| format!("failed to create export dir: {e}"))?;
    }
    fs::write(&args.out, exp.doc.as_bytes()).map_err(|e| {
        format!(
            "failed to write export markdown '{}': {e}",
            args.out.display()
        )
    })?;

    if json_mode {
        write_json_pretty(
            out,
            &json!({
                "status": "ok",
                "out": args.out.display().to_string(),
                "export_blake3": exp.export_blake3,
                "log_seq": exp.log_seq,
                "state_epoch": exp.state_epoch,
                "model_blake3": exp.model_blake3,
                "online_adaptation": exp.online_adaptation,
                "include_private": exp.include_private,
                "counts": {
                    "place_adapt": exp.place_adapt,
                    "procedures": exp.procedures,
                    "lessons": exp.lessons,
                    "beliefs": exp.beliefs,
                    "episodes_and_gists": exp.episodes_and_gists,
                },
            }),
        )?;
    } else {
        writeln!(
            out,
            "exported learned state to {} (blake3={} seq={} epoch={})",
            args.out.display(),
            exp.export_blake3,
            exp.log_seq,
            exp.state_epoch
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(0)
}
