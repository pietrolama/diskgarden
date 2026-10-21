# Decisioni di Prodotto e Design — DiskGarden (Milestone 0)

**Stato:** Definito e sottoposto a revisione (Milestone 0)  
**Data:** 2026-10-21

---

## 1. Scelta della Visualizzazione Principale (Treemap vs Radiale vs Gerarchica)

La Sezione 5 della specifica richiede di valutare una **Treemap moderna** come rappresentazione principale, confrontandola con una **vista gerarchica** e una **vista radiale (Sunburst/Ring chart)**, motivando la scelta e verificandola con un prototipo.

### 1.1 Confronto tecnico e cognitivo

| Criterio di valutazione | Treemap Squarified (con frame 2 livelli) | Vista Radiale / Sunburst (stile Filelight / Baobab) | Vista Gerarchica Pura (Albero / Tabella) |
| :--- | :--- | :--- | :--- |
| **Utilizzo dello spazio schermo (16:9 / 16:10)** | **Eccellente (100% dell'area):** l'algoritmo *Squarified* (Bruls, Huizing, van Wijk) minimizza l'aspect ratio dei rettangoli, sfruttando ogni pixel del pannello centrale. | **Basso (~65% dell'area):** un cerchio inscritto in un rettangolo spreca gli angoli e riserva il centro al nodo radice. | **Alto per il testo, basso per la percezione delle proporzioni:** richiede barre di progresso su ogni riga per confrontare i pesi. |
| **Leggibilità di etichette e valori** | **Alta:** i rettangoli con aspect ratio prossimo a 1:1 ospitano testo orizzontale su 2–3 righe (`nome`, `dimensione allocata`, `%`, `Δ snapshot`). | **Critica sugli anelli esterni:** le etichette devono essere ruotate, troncate o nascoste nei tooltip quando l'angolo scende sotto i 4°–5°. | **Eccellente:** allineamento colonnare perfetto per nomi lunghi, percorsi e numeri. |
| **Distinzione tra Directory, File singoli e Aggregati** | **Chiara (con Nested Frames):** le directory del primo livello fungono da contenitori con header dedicato; i file singoli hanno bordo continuo e icona file; gli elementi minori sono raccolti in un tile tratteggiato `"Altri N elementi"`. | **Ambigua per le foglie:** i file nella directory corrente e le sottodirectory condividono lo stesso anello concentrico; i file profondi diventano invisibili. | **Chiara solo localmente:** richiede di espandere manualmente ogni ramo per capire se una cartella pesa per 1 file da 20 GB o 500.000 file da 40 KB. |
| **Rappresentazione delle variazioni (`History` delta)** | **Immediata:** ogni tile mostra direttamente il badge testuale (`+1,4 GB` / `Nuovo`) oltre a un indicatore visivo sobrio. | **Difficile:** non c'è spazio sufficiente negli archi sottili per mostrare sia la dimensione attuale sia il delta storico senza affidarsi solo al colore. | **Ottima:** permette l'ordinamento diretto per `Δ Assoluto`, `Δ %`, `Dimensione attuale` e `Percorso`. |
| **Scalabilità su > 10.000 elementi** | **Eccellente con Tail Bucketing:** limitando il rendering ai primi $K$ elementi (o $\ge 0,3\%$ dell'area) e raggruppando la coda in un tile aggregato ispezionabile, il DOM/Canvas mantiene $< 80$ nodi attivi. | **Problematica:** la fusione degli spicchi piccoli sull'arco esterno altera la percezione della profondità. | **Buona con virtual scrolling**, ma disorientante durante la navigazione profonda. |

### 1.2 Decisione adottata per DiskGarden

Adottiamo come vista principale in **Explore** una **Vista Ibrida Sincronizzata**:
1. **Treemap Squarified Contenuta (Pannello Centrale Superiore/Principale):**
   - Implementata con l'algoritmo *Squarified Treemap* deterministico (zero dipendenze esterne opache, 100% controllabile per accessibilità da tastiera `Tab` / frecce / `Enter` / `Backspace`).
   - Mostra i figli diretti della directory attiva (e un'anteprima incorniciata dei principali nipoti per le directory più grandi), distinguendo visivamente e semanticamente:
     - **Directory** (con header, conteggio elementi e badge di eventuale errore/parzialità nel sottoalbero),
     - **Singoli file** (con badge di estensione e indicatore se *sparse* o *hard-linked*),
     - **Bucket aggregato dei file minori** (`"Altri N elementi (< 0,5%)"`), selezionabile per filtrare la tabella sottostante.
2. **Tabella/Lista Classificata Sincronizzata + Selettore Vista (Treemap / Radiale / Lista):**
   - Sotto o a fianco della Treemap, una lista ordinabile sincronizzata sulla selezione corrente garantisce che anche gli elementi troppo piccoli per avere un grande rettangolo siano sempre ricercabili, ordinabili e accessibili.
   - Il prototipo interattivo in [`prototype/index.html`](file:///home/neuron/projects/diskGarden/prototype/index.html) implementa sia la **Treemap Squarified**, sia la **Vista Radiale (Sunburst)**, sia la **Vista Tabellare Gerarchica** sullo stesso dataset per permettere la verifica diretta del confronto.

---

## 2. Regole di Prodotto per le Tre Modalità

### 2.1 Modalità `Explore` — Fotografia dello spazio
- **Doppia metrica sempre visibile:** Ogni dimensione mostra la **Dimensione Allocata** (`st_blocks * 512`, metrica primaria predefinita perché riflette i blocchi realmente consumati sul disco) e la **Dimensione Apparente** (`st_size`, byte logici). L'utente può commutare l'ordinamento tra Allocata e Apparente con un clic.
- **Visibilità esplicita degli errori di accesso:**
  - Una directory con `PermissionDenied` (`EACCES`) **non viene mai mostrata come `0 B` normale**: riceve uno stato visivo dedicato (`Accesso negato — dimensione sconosciuta`) e incrementa il contatore di avvisi della scansione e di tutte le directory antenate.
  - Se una scansione contiene anche un solo errore di accesso o viene interrotta, il badge di stato dell'header indica **`Scansione Parziale (N percorsi non accessibili)`** anziché `Completa`.

### 2.2 Modalità `History` — Evoluzione dello spazio (Differenziatore Primario)
- **Confronto senza Hash Crittografici:**
  - L'identità predefinita di confronto tra due snapshot ($T_0, T_1$) della stessa radice si basa su `rel_path_raw` + `entry_type`, arricchita da `(dev, ino, mtime_sec, mtime_nsec, apparent_bytes, allocated_bytes)`.
  - **Regola di onestà epistemica:** Se un file ha lo stesso percorso e la stessa dimensione (`Δ apparent == 0` e `Δ allocated == 0`), ma `mtime` differente, viene classificato in una categoria separata **`Metadati/Timestamp aggiornati (dimensione invariata)`** e **non** compare tra i file che hanno contribuito alla crescita dello spazio.
  - **Rilevazione euristica di possibili spostamenti/rinomine:** Se nella stessa coppia di snapshot un file viene rimosso in `path_A` e un file viene aggiunto in `path_B` con lo stesso `(dev, ino)` e la stessa dimensione, il `DiffEngine` lo segnala come *Possibile spostamento/rinomina (stesso inode)* per evitare che l'utente pensi a una nuova occupazione netta inattesa.
- **Gestione delle scansioni parziali o incompatibili:**
  - Una scansione `Cancelled` non può essere usata per confronti storici.
  - Se $T_0$ o $T_1$ è `Partial` (perché alcune sottodirectory avevano `PermissionDenied`), il confronto è consentito **con banner di avviso esplicito**, e qualsiasi sottoalbero il cui stato di accesso sia cambiato (es. `PermissionDenied` in $T_0$ $\rightarrow$ `Ok` in $T_1$) viene isolato nella sezione **`Percorsi non confrontabili in modo affidabile`**, evitando di attribuire a "crescita del disco" una directory che in precedenza era semplicemente illeggibile.

### 2.3 Modalità `Recover` — Individuazione consapevole dello spazio recuperabile
- **Zero operazioni distruttive nell'MVP:**
  - Nessun pulsante "Elimina", "Svuota" o "Pulisci ora".
  - Le sole azioni disponibili sono non distruttive: *Apri nella vista Explore*, *Mostra nella cartella (File Manager di sistema)*, *Copia percorso assoluto*, *Esporta report JSON/CSV*.
- **Qualifica obbligatoria su tre livelli per ogni suggerimento:**
  1. **Spazio occupato rilevato:** byte allocati misurati per l'elemento o sottoalbero.
  2. **Spazio potenzialmente recuperabile:** stimato solo per categorie trasparenti e ben definite:
     - *Cache utente e applicative standard* (`~/.cache`, `.cache/*`, cache browser/IDE).
     - *Artefatti di build e dipendenze rigenerabili* (`target/debug`, `target/release`, `node_modules/.cache`, `.next/cache`, `__pycache__`, `.pytest_cache`, `.gradle/caches`, `.venv` segnalato solo come ambiente ricostruibile, mai come cache sicura).
     - *File temporanei e log ruotati/crash dump* (`/tmp`, `*.tmp`, `core.*`, `*.log.1`, `*.old`).
     - *Possibili duplicati per dimensione + nome/inode* (qualificati come *Da verificare*, mai come duplicati certi senza verifica).
  3. **Spazio recuperabile effettivamente verificato:** calcolato sottraendo dallo spazio allocato:
     - I blocchi appartenenti a file con **hard link multipli (`st_nlink > 1`)** quando non tutti i link dell'inode risiedono dentro la directory candidata (perché rimuovere la directory non deallocherebbe l'inode sul filesystem).
     - Qualsiasi sottoalbero con stato `Partial` / errori di lettura non zero.

---

## 3. Direzione Visiva e Accessibilità

- **Tema predefinito:** *Botanical Slate Dark* (`#0B1114` / `#121A1F` superfici, `#E6F0EC` testo primario con contrasto $> 12:1$ WCAG AAA, accenti semantici contenuti: verde salvia/smeraldo `#2DD4BF` per stato normale/riduzione, ambra `#F59E0B` per crescita/cache da esaminare, corallo `#FB7185` per crescita critica/errori di accesso, indaco `#818CF8` per hard-link/sparse).
- **Supporto Tema Chiaro:** *Warm Alabaster Light* (`#F6F8F7` / `#FFFFFF`), commutabile istantaneamente tramite CSS Custom Properties.
- **Indipendenza dal solo colore:** Ogni stato (crescita `+`, riduzione `-`, nuovo `NEW`, rimosso `DEL`, errore `ERR`, hard-link `HL`, sparse `SP`) utilizza sempre **prefissi testuali, badge tipografici e pattern di bordo** (continuo, tratteggiato, punteggiato) oltre al colore.

---

## 4. Verifica delle Dipendenze e Scelta della Licenza

### 4.1 Dipendenze previste (Milestone 1–4)

| Componente | Libreria / Crate | Licenza verificata | Motivo dell'inclusione |
| :--- | :--- | :--- | :--- |
| **Core: Persistenza** | `rusqlite` (con `bundled`) | `MIT` | Accesso sicuro a SQLite 3 con compilazione statica della C amalgamation (zero dipendenza da `libsqlite3-dev`). |
| **Core: Serializzazione** | `serde`, `serde_json` | `MIT OR Apache-2.0` | Serializzazione tipizzata tra Core Rust, test e Presentation Layer. |
| **Core: Syscall / Unix** | `libc` (o `std::os::unix`) | `MIT OR Apache-2.0` | Metadati POSIX (`st_dev`, `st_ino`, `st_blocks`, `st_nlink`, `statvfs`). |
| **Core: Test sintetici** | `tempfile` (dev-dependency) | `MIT OR Apache-2.0` | Creazione sicura e isolamento delle directory sintetiche di test. |
| **UI: Framework** | `svelte`, `vite`, `typescript` | `MIT` | Interfaccia reattiva, leggera e tipizzata senza runtime virtual DOM pesante. |
| **UI: Treemap Engine** | Modulo interno `squarify.ts` | Licenza del progetto | Un algoritmo *Squarified Treemap* richiede ~120 righe di codice puro e testabile; evita dipendenze grafiche pesanti (`d3`/`echarts`) e garantisce controllo totale su DOM, tastiera e badge di diff. |

### 4.2 Licenza consigliata per DiskGarden
Tutte le dipendenze sopra elencate sono rilasciate sotto licenze permissive (`MIT` o `MIT OR Apache-2.0`). Pertanto il progetto non ha vincoli di copyleft imposti da terze parti e può adottare:
- **Opzione Consigliata:** **`GPL-3.0-or-later`** (standard per applicazioni desktop Linux open-source, garantisce che eventuali fork rimangano open-source) **oppure** **`MIT OR Apache-2.0`** (standard dell'ecosistema Rust). Questa scelta è elencata nel report finale per conferma da parte dell'utente.
