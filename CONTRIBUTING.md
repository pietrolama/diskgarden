# Contribuire a DiskGarden

Grazie per l'interesse nel contribuire a **DiskGarden**!

## Principi Guida per i Contributori

1. **Linux-first e zero cloud obbligatorio**: ogni funzionalità deve operare localmente sul sistema dell'utente senza account o telemetria.
2. **Separazione rigorosa tra Core e UI**:
   - La logica di scansione, aggregazione, qualifica dello spazio (`Recover`), persistenza SQLite (`SnapshotStore`) e confronto temporale (`DiffEngine`) risiede esclusivamente in `crates/disk-garden-core`.
   - L'interfaccia utente (`crates/disk-garden-app`) e la CLI (`crates/disk-garden-cli`) consumano le strutture dati di `disk-garden-core` senza duplicare calcoli o regole di dominio.
3. **Sicurezza e integrità dei dati**:
   - Nessuna cancellazione automatica dei file.
   - Nessuna invocazione di comandi shell concatenando percorsi del filesystem (`sh -c`).
   - Nessuna directory non leggibile (`PermissionDenied` o `IoError`) deve mai essere silenziosamente mostrata come vuota (`0 B`).
   - Nel confronto temporale (`DiffEngine`), non confondere mai un file con metadati modificati a parità di dimensione ($\Delta = 0\text{ B}$) con un file che ha aumentato lo spazio occupato.

## Flusso di Sviluppo

1. Esegui un fork del repository e crea un branch descrittivo (`feat/...`, `fix/...`, `docs/...`).
2. Assicurati che ogni nuova logica in `disk-garden-core` sia accompagnata da un test sintetico riproducibile in `crates/disk-garden-core/tests/`.
3. Prima di aprire una Pull Request, verifica che l'intera suite di test e `clippy` passino senza warning:
   ```bash
   cargo test
   cargo clippy --all-targets -- -D warnings
   ```
4. Documenta eventuali modifiche architetturali in `docs/ARCHITECTURE.md` o `docs/PRODUCT_DECISIONS.md`.
