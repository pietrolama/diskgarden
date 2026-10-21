use disk_garden_core::{
    build_treemap_view, compare_scans, top_directories, top_files, DiffOptions, ScanConfig,
    ScanHandle, SizeMetric, SnapshotStore,
};
use std::env;
use std::path::PathBuf;
use std::process;

fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB ({} B)", b / GIB, bytes)
    } else if b >= MIB {
        format!("{:.2} MiB ({} B)", b / MIB, bytes)
    } else if b >= KIB {
        format!("{:.2} KiB ({} B)", b / KIB, bytes)
    } else {
        format!("{} B", bytes)
    }
}

fn print_usage() {
    eprintln!(
        "Uso: disk-garden-cli [PERCORSO_DIRECTORY] [OPZIONI]\n\
         \n\
         Opzioni:\n\
         \x20 --apparent                 Ordina per dimensione apparente (st_size) invece che allocata (st_blocks)\n\
         \x20 --top <N>                  Numero di file e directory più grandi da mostrare (default: 10)\n\
         \x20 --follow-symlinks          Segue i link simbolici (con protezione anti-ciclo)\n\
         \x20 --cross-mounts             Attraversa mount point e filesystem differenti\n\
         \x20 --save-snapshot [LABEL]    Salva la scansione come snapshot nel database SQLite locale\n\
         \x20 --list-snapshots           Elenca gli snapshot salvati nel database SQLite locale\n\
         \x20 --diff <BASE_ID> <TGT_ID>  Confronta due snapshot salvati tramite DiffEngine\n\
         \x20 --json                     Emette il riepilogo strutturato in formato JSON\n"
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        process::exit(if args.is_empty() { 1 } else { 0 });
    }

    let mut root_path: Option<PathBuf> = None;
    let mut metric = SizeMetric::Allocated;
    let mut top_n: usize = 10;
    let mut follow_symlinks = false;
    let mut cross_mounts = false;
    let mut json_output = false;
    let mut save_snapshot = false;
    let mut snapshot_label: Option<String> = None;
    let mut list_snapshots = false;
    let mut diff_ids: Option<(i64, i64)> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--apparent" => metric = SizeMetric::Apparent,
            "--follow-symlinks" => follow_symlinks = true,
            "--cross-mounts" => cross_mounts = true,
            "--json" => json_output = true,
            "--list-snapshots" => list_snapshots = true,
            "--save-snapshot" => {
                save_snapshot = true;
                if i + 1 < args.len() && !args[i + 1].starts_with('-') {
                    i += 1;
                    snapshot_label = Some(args[i].clone());
                }
            }
            "--diff" => {
                if i + 2 >= args.len() {
                    eprintln!("Errore: --diff richiede <BASE_ID> <TARGET_ID>");
                    process::exit(1);
                }
                let b: i64 = args[i + 1].parse().unwrap_or(0);
                let t: i64 = args[i + 2].parse().unwrap_or(0);
                diff_ids = Some((b, t));
                i += 2;
            }
            "--top" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Errore: --top richiede un numero intero");
                    process::exit(1);
                }
                top_n = args[i].parse().unwrap_or(10);
            }
            arg if !arg.starts_with('-') && root_path.is_none() => {
                root_path = Some(PathBuf::from(arg));
            }
            other => {
                eprintln!("Argomento non riconosciuto: {}", other);
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    if list_snapshots {
        let store = SnapshotStore::open_default().unwrap_or_else(|e| {
            eprintln!("Errore apertura SnapshotStore SQLite: {}", e);
            process::exit(1);
        });
        let snaps = store.list_snapshots(root_path.as_deref()).unwrap_or_else(|e| {
            eprintln!("Errore lettura snapshot: {}", e);
            process::exit(1);
        });
        if json_output {
            println!("{}", serde_json::to_string_pretty(&snaps).unwrap());
        } else {
            println!("=== DiskGarden CLI — Snapshot Salvati ({}) ===", snaps.len());
            for s in snaps {
                println!(
                    " #{:<4} | {:<32} | {:<10} | Alloc: {:>16} | {} file, {} dir | {:?}",
                    s.id,
                    s.root_path_display,
                    s.label.as_deref().unwrap_or("-"),
                    format_bytes(s.total_allocated_bytes),
                    s.total_files_count,
                    s.total_dirs_count,
                    s.status
                );
            }
        }
        return;
    }

    if let Some((base_id, target_id)) = diff_ids {
        let store = SnapshotStore::open_default().unwrap_or_else(|e| {
            eprintln!("Errore apertura SnapshotStore SQLite: {}", e);
            process::exit(1);
        });
        let b = store.load_snapshot(base_id).unwrap_or_else(|e| {
            eprintln!("Errore caricamento snapshot base #{}: {}", base_id, e);
            process::exit(1);
        });
        let t = store.load_snapshot(target_id).unwrap_or_else(|e| {
            eprintln!("Errore caricamento snapshot target #{}: {}", target_id, e);
            process::exit(1);
        });
        let opts = DiffOptions {
            metric,
            max_items_per_category: top_n,
            base_snapshot_id: Some(b.header.id),
            target_snapshot_id: Some(t.header.id),
            base_label: b.header.label,
            target_label: t.header.label,
            ..Default::default()
        };

        let rep = compare_scans(&b.scan, &t.scan, &opts).unwrap_or_else(|e| {
            eprintln!("Errore confronto snapshot: {}", e);
            process::exit(1);
        });
        if json_output {
            println!("{}", serde_json::to_string_pretty(&rep).unwrap());
        } else {
            println!("=== DiskGarden CLI — Confronto Snapshot (#{} -> #{}) ===", base_id, target_id);
            println!("Radice:           {}", rep.root_path_display);
            println!("Base Allocato:    {}", format_bytes(rep.base_total_allocated_bytes));
            println!("Target Allocato:  {}", format_bytes(rep.target_total_allocated_bytes));
            println!("Delta Allocato:   {:+} B", rep.net_allocated_delta_bytes);
            println!("Delta File:       {:+}", rep.net_files_delta_count);
            println!("\n--- Top Directory in Variazione ---");
            for d in &rep.directory_deltas {
                println!(
                    " {:<40} | {:?} | {:+} B alloc ({:+} B app)",
                    d.rel_path_display, d.change_kind, d.delta_allocated_bytes, d.delta_apparent_bytes
                );
            }
            println!(
                "\nFile aggiunti: {} | rimossi: {} | ridimensionati: {} | modificati senza cambio dim.: {} | non confrontabili: {}",
                rep.added_files.len(),
                rep.removed_files.len(),
                rep.resized_files.len(),
                rep.metadata_only_modified_files.len(),
                rep.unreliable_paths.len()
            );
        }
        return;
    }

    let Some(root) = root_path else {
        eprintln!("Errore: specificare una directory da analizzare.");
        print_usage();
        process::exit(1);
    };

    let mut config = ScanConfig::new(root);
    config.follow_symlinks = follow_symlinks;
    config.cross_mounts = cross_mounts;

    // Run via non-blocking background ScanHandle to verify streaming progress.
    let handle = ScanHandle::spawn(config);
    let mut progress_ticks = 0usize;
    let Some(scan) = handle.wait_with_progress(|_prog| {
        progress_ticks += 1;
    }) else {
        eprintln!("Errore: il worker di scansione si è interrotto inaspettatamente.");
        process::exit(2);
    };

    if save_snapshot {
        match SnapshotStore::open_default().and_then(|st| {
            st.save_snapshot(
                &scan,
                snapshot_label.as_deref(),
                cross_mounts,
                follow_symlinks,
            )
        }) {
            Ok(hdr) => {
                if !json_output {
                    println!(
                        "Snapshot #{} salvato con successo nel database SQLite locale.",
                        hdr.id
                    );
                }
            }
            Err(e) => {
                eprintln!("Avviso: impossibile salvare lo snapshot: {}", e);
            }
        }
    }

    if json_output {
        let top_f: Vec<_> = top_files(&scan, top_n, metric, false)
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "rel_path": f.rel_path_display,
                    "allocated_bytes": f.allocated_bytes,
                    "apparent_bytes": f.apparent_bytes,
                    "is_sparse": f.is_sparse(),
                    "nlink": f.nlink,
                })
            })
            .collect();
        let top_d: Vec<_> = top_directories(&scan, top_n, metric)
            .into_iter()
            .map(|d| {
                serde_json::json!({
                    "rel_path": d.rel_path_display,
                    "subtree_allocated_bytes": d.subtree_allocated_bytes,
                    "subtree_apparent_bytes": d.subtree_apparent_bytes,
                    "subtree_files_count": d.subtree_files_count,
                    "subtree_dirs_count": d.subtree_dirs_count,
                    "access_status": d.access_status,
                })
            })
            .collect();
        let out = serde_json::json!({
            "root_path": scan.root_path_display,
            "status": scan.status,
            "duration_ms": scan.duration_ms,
            "progress_events_received": progress_ticks,
            "total_allocated_bytes": scan.total_allocated_bytes,
            "total_apparent_bytes": scan.total_apparent_bytes,
            "total_files_count": scan.total_files_count,
            "total_dirs_count": scan.total_dirs_count,
            "total_symlinks_count": scan.total_symlinks_count,
            "hardlink_duplicate_files_count": scan.hardlink_duplicate_files_count,
            "hardlink_saved_allocated_bytes": scan.hardlink_saved_allocated_bytes,
            "permission_denied_count": scan.permission_denied_count,
            "io_errors_count": scan.io_errors_count,
            "excluded_mounts_count": scan.excluded_mounts_count,
            "top_directories": top_d,
            "top_files": top_f,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return;
    }

    println!("=== DiskGarden CLI — Riepilogo Scansione ===");
    println!("Radice:              {}", scan.root_path_display);
    println!("Stato:               {:?}", scan.status);
    println!("Durata:              {} ms (eventi progresso: {})", scan.duration_ms, progress_ticks);
    println!("Spazio Allocato:     {}", format_bytes(scan.total_allocated_bytes));
    println!("Spazio Apparente:    {}", format_bytes(scan.total_apparent_bytes));
    println!(
        "Conteggi:            {} file, {} sottodirectory, {} symlink",
        scan.total_files_count, scan.total_dirs_count, scan.total_symlinks_count
    );
    if scan.hardlink_duplicate_files_count > 0 {
        println!(
            "Hard Link deduplic.: {} link aggiuntivi ({} allocati risparmiati dal doppio conteggio)",
            scan.hardlink_duplicate_files_count,
            format_bytes(scan.hardlink_saved_allocated_bytes)
        );
    }
    if scan.permission_denied_count > 0 || scan.io_errors_count > 0 {
        println!(
            "Avvisi Accesso/I/O:  {} permessi negati, {} errori I/O ({} dir non leggibili)",
            scan.permission_denied_count, scan.io_errors_count, scan.unreadable_dirs_count
        );
    }

    println!("\n--- Top {} Directory ({:?}) ---", top_n, metric);
    for (idx, d) in top_directories(&scan, top_n, metric).iter().enumerate() {
        println!(
            "{:>2}. {:<42} | Alloc: {:>18} | App: {:>18} | {} file, {} dir | {:?}",
            idx + 1,
            d.rel_path_display,
            format_bytes(d.subtree_allocated_bytes),
            format_bytes(d.subtree_apparent_bytes),
            d.subtree_files_count,
            d.subtree_dirs_count,
            d.access_status
        );
    }

    println!("\n--- Top {} File ({:?}) ---", top_n, metric);
    for (idx, f) in top_files(&scan, top_n, metric, false).iter().enumerate() {
        let mut flags = Vec::new();
        if f.is_sparse() {
            flags.push("SPARSE".to_string());
        }
        if f.nlink > 1 {
            flags.push(format!("HL({}/{})", f.shared_link_paths_in_scan, f.nlink));
        }
        let flag_str = if flags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", flags.join(", "))
        };
        println!(
            "{:>2}. {:<42} | Alloc: {:>18} | App: {:>18}{}",
            idx + 1,
            f.rel_path_display,
            format_bytes(f.allocated_bytes),
            format_bytes(f.apparent_bytes),
            flag_str
        );
    }

    if let Some(root_id) = scan.root_id {
        let tiles = build_treemap_view(&scan, root_id, metric, 12, 0.005);
        println!("\n--- Treemap Root Tiles ({} tile preparati) ---", tiles.len());
        for t in tiles {
            println!(
                " - {:<36} ({:>5.1}%) | Alloc: {} | App: {}",
                t.label,
                t.fraction_of_parent * 100.0,
                format_bytes(t.allocated_bytes),
                format_bytes(t.apparent_bytes)
            );
        }
    }
}
