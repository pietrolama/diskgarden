# 🌱 DiskGarden

**Esplorazione spaziale, memoria temporale e recupero consapevole dello spazio su disco per Linux.**

[![License: GPL v3+](https://img.shields.io/badge/License-GPL%20v3%2B-teal.svg)](LICENSE)
[![Platform: Linux](https://img.shields.io/badge/Platform-Linux-blue.svg)](#)
[![Version: 0.1.0-alpha.1](https://img.shields.io/badge/Release-v0.1.0--alpha.1-2dd4bf.svg)](#)

DiskGarden è un'applicazione desktop Linux open-source progettata per rispondere non solo alla domanda *"Quali file e directory occupano più spazio?"*, ma anche a:

- **Dove sta crescendo il mio disco nel tempo?**
- **Quali directory e file sono cambiati dall'ultima scansione?**
- **Quanto spazio è realmente recuperabile su disco tenendo conto di hard link, file sparsi e permessi?**
- **Come posso esplorare milioni di file rapidamente senza bloccare l'interfaccia?**

---

## Caratteristiche principali

### 1. `Explore` — Fotografia dello spazio
- **Scansione predefinita dell'intero disco (`/`)** con barra dei volumi fisici montati (`statvfs` + `/proc/mounts`) e scorciatoia rapida per `$HOME`.
- **Motore di scansione multi-core non bloccante** in Rust (`lstat` diretto tramite `symlink_metadata`, deduplicazione hard link tramite `(st_dev, st_ino)`, esclusione automatica di pseudo-filesystem `/proc`, `/sys`, `/dev`, `/run`, protezione anti-ciclo sui symlink).
- **Doppia metrica trasparente**:
  - **Dimensione Allocata (`st_blocks * 512`)**: spazio fisico realmente occupato sui blocchi del filesystem.
  - **Dimensione Apparente (`st_size`)**: lunghezza logica del file (evidenziazione immediata dei **file sparsi**).
- **Treemap Squarified a 2 livelli con Tail Bucketing automatico**, vista **Radiale** e vista **Per Estensione**: anche in directory con centinaia di migliaia di piccoli file, gli elementi minori vengono raggruppati in un bucket ispezionabile senza mai generare rettangoli illeggibili da 1px.
- **Gestione rigorosa degli errori di accesso**: le directory con `PermissionDenied` o `IoError` producono una scansione `Partial` chiaramente segnalata e non vengono mai mostrate come cartelle vuote da `0 B`.

### 2. `History` — Evoluzione dello spazio nel tempo (`SnapshotStore` + `DiffEngine`)
- **Snapshot locali su SQLite (`WAL`)** conformi allo schema [`docs/SCHEMA_V1.sql`](docs/SCHEMA_V1.sql), salvati in `~/.local/share/disk-garden/snapshots.db`.
- **Confronto diretto (`DiffEngine`)** tra uno snapshot salvato e la scansione corrente (o tra due snapshot qualsiasi della stessa radice) senza necessità di ricalcolare hash crittografici dell'intero disco:
  - **Directory in crescita o riduzione** ordinate per variazione con segno (`+Δ`), variazione assoluta (`|Δ|`), dimensione attuale o percorso.
  - **File esistenti ridimensionati ($\Delta \neq 0\text{ B}$)**, distinguendo i file cresciuti/ridotti sul posto da quelli con inode sostituito (`replaced_inode`).
  - **File aggiunti e file rimossi**.
  - **File modificati nei metadati a parità di dimensione ($\Delta = 0\text{ B}$)**: separati rigorosamente dai file cresciuti per non confondere una modifica a timestamp/inode con una crescita di spazio.
  - **Percorsi non confrontabili con certezza (`unreliable_paths`)**: isola le directory non accessibili (`PermissionDenied`, `IoError`) evitando falsi positivi di file aggiunti o rimossi.

### 3. `Recover` — Individuazione consapevole dello spazio recuperabile
- **Nessuna cancellazione automatica o distruttiva**.
- Qualificazione a **3 livelli** per ogni candidato:
  1. **Spazio occupato rilevato**: byte fisici allocati nel sottoalbero.
  2. **Spazio potenzialmente recuperabile**: stimato tramite regole trasparenti e documentate (`~/.cache`, artefatti di build rigenerabili come `target/debug`, `node_modules/.cache`, log ruotati). I file utente, documenti e sorgenti non vengono mai etichettati come spazzatura.
  3. **Spazio recuperabile verificato su disco**: byte che verrebbero effettivamente liberati dal filesystem (sottraendo gli inode con **hard link esterni** `st_nlink > link_interni` che resterebbero allocati altrove).

---

## Architettura del Repository

Il progetto è organizzato come workspace Cargo modulare:

```text
diskGarden/
├── crates/
│   ├── disk-garden-core/   # Libreria core: Scanner, Analyzer, SnapshotStore (SQLite), DiffEngine
│   ├── disk-garden-app/    # Applicazione desktop Linux (UI + coordinatore stato locale)
│   └── disk-garden-cli/    # Strumento da riga di comando (scansione, snapshot, diff, export JSON)
├── docs/
│   ├── COMPETITIVE_ANALYSIS.md  # Confronto verificato con QDirStat, Filelight, Baobab, Dua, Ncdu
│   ├── ARCHITECTURE.md          # Architettura tecnica e separazione dei componenti
│   ├── PRODUCT_DECISIONS.md     # Decisioni di prodotto e regole di dominio
│   ├── SCHEMA_V1.sql            # Schema database SQLite v1 per gli snapshot
│   └── TESTING.md               # Guida alla suite di test sintetici e di integrazione
├── packaging/              # File .desktop, AppStream metainfo e installer utente locale
└── prototype/              # Prototipo statico iniziale (Milestone 0)
```

---

## Requisiti e Compilazione (Linux)

### Dipendenze di sistema
- **Rust toolchain** (`rustc >= 1.80`, `cargo`)
- **SQLite 3 runtime** (`libsqlite3.so.0`, preinstallato su tutte le principali distribuzioni Linux)
- Per la finestra desktop nativa: `qml6` con `QtWebEngine` (`qml6-module-qtwebengine`) oppure qualsiasi browser Chromium/Brave/xdg-open in modalità finestra app.

### Avvio rapido dell'applicazione desktop
```bash
# Avvia DiskGarden (di default analizza l'intero disco "/")
cargo run -p disk-garden-app

# Oppure avvia su una directory specifica
cargo run -p disk-garden-app -- /percorso/cartella
```

### Utilizzo della CLI (`disk-garden-cli`)
```bash
# Scansione rapida da terminale con top 10 directory e file
cargo run -p disk-garden-cli -- /home/utente --top 10

# Salva la scansione come snapshot SQLite con etichetta
cargo run -p disk-garden-cli -- /home/utente --save-snapshot "Prima della pulizia"

# Elenca gli snapshot salvati
cargo run -p disk-garden-cli -- --list-snapshots

# Confronta due snapshot salvati (#1 vs #2)
cargo run -p disk-garden-cli -- --diff 1 2

# Esporta output strutturato in JSON
cargo run -p disk-garden-cli -- /home/utente --json
```

### Installazione nel menu applicazioni utente (senza `sudo`)
```bash
./packaging/install-local.sh
```

---

## Scorciatoie da tastiera (Interfaccia Desktop)

| Tasto | Azione |
|---|---|
| `Invio` | Entra nella directory selezionata |
| `Backspace` o `Alt + ↑` | Sali alla directory genitore |
| `/` | Focalizza la barra di ricerca istantanea |
| `Esc` | Chiudi la finestra modale o rimuovi il focus dalla ricerca |

---

## Test e Qualità del Codice

```bash
# Esegue tutti i test unitari, sintetici del filesystem e di integrazione API/SQLite
cargo test

# Verifica assenza totale di warning statici
cargo clippy --all-targets -- -D warnings
```

Consulta [`docs/TESTING.md`](docs/TESTING.md) per i dettagli sui casi di test sintetici (file sparsi, hard link multipli/esterni, cicli di symlink, permessi negati, percorsi non-UTF-8 e diff temporali).

---

## Licenza

Distribuito sotto licenza **GNU General Public License v3.0 or later (`GPL-3.0-or-later`)**. Consulta il file [`LICENSE`](LICENSE) per il testo completo.
