# Strategia e Suite di Test (`docs/TESTING.md`)

DiskGarden adotta una strategia di verifica basata su **alberi di directory sintetici deterministici** creati in directory temporanee isolate (`tempfile::TempDir`), affiancata da test di integrazione per la persistenza SQLite e le API dell'applicazione desktop.

## Esecuzione della Suite

```bash
# Esegue tutti i 19 test automatici del workspace
cargo test --workspace

# Esegue solo i test sintetici dello Scanner, dell'Analyzer e del confronto con GNU du
cargo test -p disk-garden-core --test scanner_synthetic_tests

# Esegue solo i test e benchmark di SnapshotStore SQLite e DiffEngine
cargo test -p disk-garden-core --test snapshot_diff_tests -- --nocapture

# Esegue i test di integrazione end-to-end e di HTTP Hardening (401/403)
cargo test -p disk-garden-app --test ui_api_integration_tests

# Verifica statica rigorosa (zero warning ammessi)
cargo clippy --workspace --all-targets -- -D warnings
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
12. **`test_realistic_comparison_with_gnu_du_allocated_apparent_and_hardlinks`**: confronta al singolo byte i risultati di DiskGarden con `/usr/bin/du` sia su un albero controllato (con hard link e file sparsi) sia sulla directory reale `crates/` del repository:
    - **Spazio allocato deduplicato (`du -s -B1`)**: GNU `du` somma i blocchi fisici (`st_blocks * 512`) dei file deduplicati per hard link **più** i blocchi di metadati degli inode delle directory e dei symlink. In DiskGarden `scan.total_allocated_bytes` misura il puro contenuto dei file (così una directory vuota mostra `0 B`), mentre ogni `EntryNode` conserva il proprio `allocated_bytes`; vale quindi l'identità esatta al byte:
      $$\text{du -s -B1} = \text{scan.total\_allocated\_bytes} + \sum_{e \in \text{dir} \cup \text{symlink}} e.\text{allocated\_bytes}$$
    - **Spazio allocato non deduplicato (`du -s -B1 -l`)**: aggiungendo `scan.hardlink_saved_allocated_bytes` si ottiene l'identità esatta al byte con `du -s -B1 -l`.
    - **Spazio apparente (`du -s -B1 --apparent-size`)**: in GNU `du` (coreutils 9.x) `--apparent-size` non somma la lunghezza dell'inode delle directory ma solo `st_size` dei file (e symlink), coincidendo quindi direttamente con `scan.total_apparent_bytes + symlink_apparent`.

### 2. SnapshotStore SQLite & DiffEngine (`crates/disk-garden-core/tests/snapshot_diff_tests.rs`)
1. **`test_snapshot_store_save_list_load_and_delete`**: verifica il ciclo completo di salvataggio su SQLite (`SCHEMA_V1.sql`), filtro per radice, ricostruzione dell'albero `ScanResult` e cancellazione `ON DELETE CASCADE`.
2. **`test_incomplete_or_mismatched_scans_rejected`**: verifica che scansioni `Cancelled`/`Failed` non possano essere salvate né confrontate come complete, e che radici differenti producano `DiffError::RootPathMismatch`.
3. **`test_temporal_diff_added_removed_resized_metadata_only_and_unreliable_paths`**:
   - Verifica la rilevazione precisa di file aggiunti, rimossi e ridimensionati.
   - Verifica che un file riscritto atomicamente con la **stessa identica dimensione** ($\Delta = 0\text{ B}$, nuovo inode) finisca in `metadata_only_modified_files` e **mai** tra i file cresciuti (`resized_files`).
   - Verifica che una sottodirectory divenuta inaccessibile (`chmod 000`) finisca in `unreliable_paths` senza che i suoi file interni vengano falsamente dichiarati rimossi.
4. **`test_large_synthetic_tree_temporal_diff_benchmark_and_accuracy`**:
   - Costruisce un albero sintetico di **5.000 file su 100 directory** (`5.101` nodi totali), salva lo snapshot `T0` su SQLite, applica mutazioni note (25 file cresciuti di `+4.096 B`, 10 file ridotti di `-512 B`, 40 file aggiunti da `2.048 B`, 15 file rimossi da `1.024 B`, 12 file riscritti atomicamente via `rename` a parità di dimensione `1.024 B`), salva lo snapshot `T1` e misura tempi e accuratezza del diff:
     - **Scansione `T0` / `T1`**: `~10–17 ms`
     - **Salvataggio SQLite (`WAL` + prepared batch transaction)**: `~65–70 ms` per snapshot (`~1,2 MiB` per 5.101 nodi)
     - **Caricamento da SQLite (2 snapshot)**: `~30 ms` complessivi (`~15 ms` ciascuno)
     - **Esecuzione `DiffEngine::compare_scans`**: **`~2 ms`**, con accuratezza matematica del 100% (`net_apparent_delta_bytes == +163.840 B`, `40` aggiunti, `15` rimossi, `35` ridimensionati, `12` modificati a parità di byte).

### 3. Integrazione Desktop API & HTTP Hardening (`crates/disk-garden-app/tests/ui_api_integration_tests.rs`)
1. **`test_http_hardening_rejects_missing_or_wrong_token_and_invalid_origins_and_hosts`**: verifica che richieste prive di `X-DiskGarden-Token` o con token errato ricevano `401 Unauthorized`, e che richieste con `Origin`, `Referer` o `Host` estranei ricevano `403 Forbidden`.
2. **`test_milestone_2_progressive_navigation_treemap_bucketing_search_and_recover`**: verifica la navigazione progressiva via HTTP/JSON dalla radice fino al singolo file, il tail bucketing su 120 file piccoli, la ricerca/filtro e il report `Recover` a 3 livelli.
3. **`test_milestone_3_history_snapshots_and_diff_api`**: verifica l'intero flusso `History` tramite gli endpoint `/api/snapshots/save`, `/api/snapshots`, `/api/history/diff`, `/api/snapshots/load` e `/api/snapshots/delete`.
