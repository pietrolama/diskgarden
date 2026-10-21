# Strategia e Suite di Test (`docs/TESTING.md`)

DiskGarden adotta una strategia di verifica basata su **alberi di directory sintetici deterministici** creati in directory temporanee isolate (`tempfile::TempDir`), affiancata da test di integrazione per la persistenza SQLite e le API dell'applicazione desktop.

## Esecuzione della Suite

```bash
# Esegue tutti i 16 test automatici del workspace
cargo test

# Esegue solo i test sintetici dello Scanner e dell'Analyzer (Milestone 1)
cargo test -p disk-garden-core --test scanner_synthetic_tests

# Esegue solo i test di SnapshotStore SQLite e DiffEngine (Milestone 3)
cargo test -p disk-garden-core --test snapshot_diff_tests

# Esegue i test di integrazione end-to-end del server UI/API (Milestone 2 & 3)
cargo test -p disk-garden-app --test ui_api_integration_tests

# Verifica statica rigorosa (zero warning ammessi)
cargo clippy --all-targets -- -D warnings
```

---

## Copertura dei Casi Limite

### 1. Scanner & Analyzer (`crates/disk-garden-core/tests/scanner_synthetic_tests.rs`)
1. **`test_empty_directory`**: verifica che una directory vuota produca `Completed`, `0` file, `0` sottodirectory e `0` byte di contenuto.
2. **`test_nested_hierarchy_totals_and_sorting`**: verifica l'aggregazione bottom-up dei sottoalberi su più livelli, l'estrazione `top_files` / `top_directories`, il filtraggio e il tail bucketing della Treemap.
3. **`test_sparse_file_apparent_vs_allocated`**: crea un file sparso reale tramite `ftruncate` (dimensione logica apparente `1 MiB`, blocchi fisici allocati `0 B`) e verifica che `apparent_bytes` e `allocated_bytes` siano entrambi preservati e che `is_sparse()` sia `true`.
4. **`test_hardlinks_deduplication_and_external_link_detection`**:
   - Crea hard link multipli interni alla radice scansionata e verifica che lo spazio allocato venga contato **una sola volta** nel totale (evitando sovrastime).
   - Crea un hard link che punta *all'esterno* della radice scansionata (`st_nlink > shared_link_paths_in_scan`) e verifica che la qualificazione `Recover` riduca lo `verified_recoverable_bytes` a `0 B` per quell'inode.
5. **`test_symlinks_and_mutual_symlink_cycles`**: crea due directory che puntano l'una all'altra tramite symlink (`cycle_a/to_b -> cycle_b` e `cycle_b/to_a -> cycle_a`) e verifica che anche con `follow_symlinks = true` lo scanner rilevi il ciclo tramite `(st_dev, st_ino)` e termini senza loop infiniti.
6. **`test_permission_denied_is_partial_not_empty_directory`**: imposta `chmod 000` su una sottodirectory e verifica che lo stato della scansione diventi `ScanStatus::Partial` e il nodo abbia `AccessStatus::PermissionDenied` (mai confuso con una directory vuota).
7. **`test_unicode_special_chars_and_invalid_utf8_paths`**: verifica la gestione di nomi contenenti emoji, spazi, simboli shell (`$`, `` ` ``, `'`, `"`) e sequenze di byte POSIX **non-UTF-8** (`0xFF`, `0xFE`), preservando `rel_path_raw` esatto e marcando `is_valid_utf8 = false`.
8. **`test_cancellation_and_background_scan_handle`**: verifica l'annullamento cooperativo tramite `CancelToken` e la ricezione non bloccante degli eventi `ScanEvent::Progress` e `ScanEvent::Finished`.
9. **`test_mount_boundary_exclusion`**: verifica il rispetto dei confini di mount point (`st_dev`).
10. **`test_invalid_root_paths_fail_gracefully`**: verifica la gestione pulita (`ScanStatus::Failed`) di percorsi inesistenti o file singoli passati come radice.
11. **`test_vanished_directory_during_scan_is_handled_gracefully`**: verifica la resilienza a file/directory che scompaiono durante la scansione (`ENOENT` $\rightarrow$ `AccessStatus::Vanished`).

### 2. SnapshotStore SQLite & DiffEngine (`crates/disk-garden-core/tests/snapshot_diff_tests.rs`)
1. **`test_snapshot_store_save_list_load_and_delete`**: verifica il ciclo completo di salvataggio su SQLite (`SCHEMA_V1.sql`), filtro per radice, ricostruzione dell'albero `ScanResult` e cancellazione `ON DELETE CASCADE`.
2. **`test_incomplete_or_mismatched_scans_rejected`**: verifica che scansioni `Cancelled`/`Failed` non possano essere salvate né confrontate come complete, e che radici differenti producano `DiffError::RootPathMismatch`.
3. **`test_temporal_diff_added_removed_resized_metadata_only_and_unreliable_paths`**:
   - Verifica la rilevazione precisa di file aggiunti, rimossi e ridimensionati.
   - Verifica che un file riscritto atomicamente con la **stessa identica dimensione** ($\Delta = 0\text{ B}$, nuovo inode) finisca in `metadata_only_modified_files` e **mai** tra i file cresciuti (`resized_files`).
   - Verifica che una sottodirectory divenuta inaccessibile (`chmod 000`) finisca in `unreliable_paths` senza che i suoi file interni vengano falsamente dichiarati rimossi.

### 3. Integrazione Desktop API (`crates/disk-garden-app/tests/ui_api_integration_tests.rs`)
1. **`test_milestone_2_progressive_navigation_treemap_bucketing_search_and_recover`**: verifica la navigazione progressiva via HTTP/JSON dalla radice fino al singolo file, il tail bucketing su 120 file piccoli, la ricerca/filtro e il report `Recover` a 3 livelli.
2. **`test_milestone_3_history_snapshots_and_diff_api`**: verifica l'intero flusso `History` tramite gli endpoint `/api/snapshots/save`, `/api/snapshots`, `/api/history/diff`, `/api/snapshots/load` e `/api/snapshots/delete`.
