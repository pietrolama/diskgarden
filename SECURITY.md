# Politica di Sicurezza (SECURITY.md)

DiskGarden interagisce direttamente con il filesystem locale dell'utente, inclusi percorsi con caratteri arbitrari, link simbolici, mount point multipli e directory con permessi ristretti. La sicurezza operativa è un requisito architetturale primario.

## Garanzie di Sicurezza Implementate

1. **Nessuna operazione distruttiva automatica**:
   - DiskGarden opera in sola lettura sul filesystem analizzato (l'unica scrittura avviene nel proprio database SQLite locale `~/.local/share/disk-garden/snapshots.db`).
   - La modalità `Recover` fornisce esclusivamente suggerimenti qualificati per la revisione consapevole dell'utente.
2. **Zero Command Injection sui percorsi del filesystem**:
   - I percorsi su Linux possono contenere qualsiasi byte diverso da `\0`, inclusi spazi, virgolette, backtick, newline, sequenze non-UTF-8 e caratteri di controllo.
   - DiskGarden **non costruisce mai comandi shell** a partire dai percorsi dei file. L'apertura del file manager esterno utilizza `std::process::Command::new("xdg-open").arg(&dir_path)` passando l'`OsStr` direttamente alla syscall `execve` senza alcuna interpolazione di shell.
3. **Escape rigoroso nell'interfaccia utente**:
   - Tutti i nomi e percorsi visualizzati nella UI vengono sanitizzati tramite `escapeHtml` o `textContent` per prevenire qualsiasi iniezione HTML/DOM da nomi di file malevoli (es. un file chiamato `<img src=x onerror=...>`).
4. **Protezione da cicli infiniti e pseudo-filesystem**:
   - Di default (`follow_symlinks = false`, `cross_mounts = false`), lo scanner usa `symlink_metadata` (`lstat`) e non attraversa link simbolici né confini di mount point (`st_dev != root_dev`).
   - Quando `follow_symlinks = true`, lo scanner traccia il set delle directory antenate attive `(st_dev, st_ino)` e interrompe immediatamente qualsiasi ciclo simbolico (`AccessStatus::SymlinkCycleSkipped`).
   - Durante la scansione di `/`, gli pseudo-filesystem del kernel Linux (`/proc`, `/sys`, `/dev`, `/run`) vengono sempre esclusi alla radice anche se l'utente abilita `cross_mounts`.
5. **Integrità delle transazioni SQLite**:
   - Il salvataggio degli snapshot avviene in una transazione `IMMEDIATE` con stato iniziale `'saving'`, promosso a `'completed'` o `'partial'` solo al termine dell'inserimento di tutti i nodi. Eventuali snapshot interrotti da un crash vengono isolati all'avvio successivo e mai usati come baseline di confronto.

## Segnalazione di Vulnerabilità

Se individui un bug di sicurezza (es. traversamento non autorizzato, denial-of-service tramite alberi di directory appositamente costruiti o problemi di escaping), apri una segnalazione privata su GitHub Security Advisories del repository https://github.com/pietrolama/diskgarden.
