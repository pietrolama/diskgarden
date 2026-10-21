# Analisi Competitiva — DiskGarden (Milestone 0)

**Stato:** Completato (Milestone 0)  
**Data:** 2026-10-21  
**Scopo:** Verificare gli strumenti esistenti di analisi dello spazio su disco per Linux, esaminarne stack, licenze, funzionalità e limiti, e definire il posizionamento concreto e verificabile di DiskGarden.

---

## 1. Metodologia di verifica

Prima di progettare l'architettura e le funzionalità di DiskGarden, sono stati analizzati i quattro strumenti grafici di riferimento richiesti dalla specifica (**QDirStat**, **eDirStat**, **Filelight**, **Baobab**) insieme a tre strumenti complementari rilevanti nell'ecosistema Linux (**Duc**, **ncdu** + `ncdu-compare`, **dua-cli / gdu**).

L'analisi si concentra su cinque dimensioni:
1. **Rappresentazione spaziale e navigazione** (Treemap, Sunburst/Ring chart, Tree list, TUI).
2. **Memoria temporale** (salvataggio snapshot, formato dati, confronto automatico tra scansioni).
3. **Individuazione dello spazio recuperabile** (pulizia, cache, duplicati, appartenenza a pacchetti).
4. **Correttezza sul filesystem Linux** (dimensione apparente vs allocata, hard link, file sparsi, mount point, permessi negati, percorsi non-UTF-8).
5. **Stack tecnologico, stato di manutenzione e licenza**.

---

## 2. Analisi dettagliata delle alternative

### 2.1 QDirStat
- **Repository:** `https://github.com/shundhammer/qdirstat` (autore: Stefan Hundhammer, già autore di KDirStat).
- **Stack tecnologico:** C++ con Qt (aggiornato a Qt6), senza dipendenze dalle librerie KDE Frameworks; script companion in Perl (`qdirstat-cache-writer`) per scansioni headless su server.
- **Licenza:** `GPL-2.0-or-later`.
- **Stato di sviluppo:** Attivamente mantenuto, molto maturo e stabile.
- **Funzionalità principali:**
  - Vista combinata: albero gerarchico multipannello con colonne metriche + **Treemap cushioned** (con ombreggiatura 2.5D) in basso.
  - Integrazione con i package manager Linux (`dpkg`, `rpm`, `pacman`) per identificare a quale pacchetto di sistema appartiene un file o quali file in directory di sistema non appartengono a nessun pacchetto (*Unpackaged Files view*).
  - Viste statistiche per estensione/MIME, istogramma delle dimensioni e anzianità dei file (*File Age view*).
  - Sistema di *Cleanup Actions* configurabili dall'utente (anche tramite shell).
  - Salvataggio e caricamento di file di cache testuali compressi con gzip (`.qdirstat.cache.gz`, formato 1.0 e 2.0 con tipo `D`/`F`/`L`, dimensione, `mtime`, `uid`, `gid`, permessi).
- **Limiti verificati:**
  1. **Assenza di un motore di confronto (Diff Engine) tra snapshot:** QDirStat permette di caricare un singolo file di cache alla volta per evitare la riscansione, ma non offre alcun confronto automatico tra due snapshot ($T_0$ vs $T_1$) per individuare quali directory sono cresciute, quali file sono stati aggiunti/rimossi o quali hanno aumentato dimensione.
  2. **Interfaccia densa e orientata all'amministrazione di sistema:** Molto potente per sysadmin, ma complessa per utenti desktop o sviluppatori che cercano una risposta immediata alla domanda *"Cosa ha riempito il disco rispetto alla settimana scorsa?"*.
  3. **Rischio nelle azioni di pulizia personalizzate:** Le cleanup actions eseguono comandi configurati o shell; DiskGarden per scelta architetturale esclude qualsiasi esecuzione di comandi shell costruiti dai percorsi e qualsiasi cancellazione nel primo MVP.

---

### 2.2 eDirStat
- **Repository:** `https://github.com/xangelix/edirstat`
- **Stack tecnologico:** Rust con interfaccia immediate-mode `egui`.
- **Licenza:** `MIT` (codice sorgente open source; binari precompilati distribuiti anche commercialmente).
- **Stato di sviluppo:** Progetto moderno in attivo sviluppo.
- **Funzionalità principali:**
  - Scansione multithread ad alte prestazioni (work-stealing e arena zero-copy).
  - Visualizzazione **Treemap** interattiva con breadcrumb e grafici analitici (distribuzione dimensioni, scatter plot età vs dimensione).
  - Pipeline di deduplicazione crittografica a 7 stadi (dimensione, pre-hash parziale, SHA-256 completo) per individuare file duplicati.
  - Formato snapshot proprietario compresso con Zstd (`.edst`) a layout colonnare, progettato per caricamento rapido.
- **Limiti verificati:**
  1. **Snapshot statici senza Diff Engine temporale:** Nonostante il supporto al salvataggio/caricamento rapido di file `.edst`, eDirStat usa gli snapshot come fotografia statica (*point-in-time*), senza un motore di diff strutturato che calcoli delta netti per sottoalbero, file cresciuti o percorsi non confrontabili tra due scansioni.
  2. **Focus sulla deduplicazione crittografica pesante:** La ricerca del recupero spazio punta fortemente sull'hashing dei contenuti anziché sulla comprensione dell'evoluzione temporale e sull'analisi non distruttiva delle directory di lavoro/cache.
  3. **UI Immediate-Mode (`egui`):** Ottima per il rendering GPU diretto, ma meno adatta alla navigazione da tastiera standard, all'accessibilità (screen reader / focus ring semantici) e alla leggibilità tipografica di tabelle comparative complesse.

---

### 2.3 Filelight (KDE Gear)
- **Repository:** `https://invent.kde.org/utilities/filelight`
- **Stack tecnologico:** C++ / Qt6 / Kirigami / KDE Frameworks.
- **Licenza:** `GPL-2.0-only OR GPL-3.0-only`.
- **Stato di sviluppo:** Attivamente mantenuto all'interno del ciclo di rilascio di KDE Gear.
- **Funzionalità principali:**
  - Visualizzazione radiale (**Sunburst / anelli concentrici**) in cui ogni anello rappresenta un livello di profondità e l'angolo è proporzionale alla dimensione.
  - Navigazione zoomabile cliccando sui settori circolari, con tooltip dettagliati al passaggio del mouse.
  - Integrazione diretta con Dolphin e il cestino di KDE Plasma.
- **Limiti verificati:**
  1. **Nessuna persistenza o cronologia:** Non salva snapshot né consente alcun confronto nel tempo.
  2. **Degrado geometrico del grafico Sunburst su alberi profondi:** Nelle gerarchie tipiche degli sviluppatori (es. `node_modules`, `.cargo`, build C++/Rust, repository profondi), gli anelli esterni si frammentano in settori angolari sottilissimi (< 1°), impossibili da selezionare o etichettare, mentre gli elementi piccoli vengono omessi o fusi senza possibilità di ispezione tabellare.
  3. **Scarsa visibilità dei singoli file grandi annidati:** Il grafico radiale privilegia i contenitori vicini alla radice; un singolo file di 8 GB al livello 6 di profondità occupa lo stesso angolo ma è relegato sull'anello periferico.

---

### 2.4 Baobab (GNOME Disk Usage Analyzer)
- **Repository:** `https://gitlab.gnome.org/GNOME/baobab`
- **Stack tecnologico:** Vala (>95%) + C, GTK4 e Libadwaita.
- **Licenza:** `GPL-2.0-or-later`.
- **Stato di sviluppo:** Attivamente mantenuto come applicazione core di GNOME.
- **Funzionalità principali:**
  - Interfaccia minimale conforme alle GNOME HIG.
  - Doppio grafico commutabile: **Ring Chart (Sunburst)** e **Treemap**, affiancato da un albero gerarchico.
  - Supporto ai mount remoti tramite GVfs (SFTP, SMB, WebDAV).
- **Limiti verificati:**
  1. **Nessun salvataggio snapshot e nessun confronto storico:** Ogni avvio o analisi è effimera; chiudendo la finestra i dati raccolti vanno persi.
  2. **Assenza di ricerca e filtri:** Nelle versioni moderne di Baobab la funzione di ricerca interna è stata rimossa per mantenere l'interfaccia minimale; non è possibile filtrare per nome, tipo, dimensione minima o visualizzare la classifica globale ("Top N file più grandi") trasversale alle cartelle.
  3. **Nessuna distinzione chiara tra spazio apparente, spazio allocato e spazio recuperabile:** I file sparsi e gli hard link non sono esplicitati nel pannello di ispezione.

---

### 2.5 Altri strumenti pertinenti (Duc, ncdu, dua-cli / gdu)

| Strumento | Stack & Licenza | Punti di forza | Limiti rispetto a DiskGarden |
| :--- | :--- | :--- | :--- |
| **Duc** (`zevv/duc`) | C; `LGPL-3.0`; DB backend (LMDB, TokyoCabinet, SQLite, LevelDB) | Indicizza enormi filesystem in un DB locale; supporta UI ncurses, GUI X11/OpenGL e web. | L'indicizzazione (`duc index`) sovrascrive il path nel DB di default; non ha un Diff Engine grafico né ispezione a livello di singoli file cresciuti o gestione strutturata di scansioni parziali. |
| **ncdu** + `ncdu-compare` | C (v1) / Zig (v2); `MIT` | Standard de facto da terminale; supporta export/import JSON (`-o` / `-f`). | È un tool TUI da terminale; il confronto richiede script esterni di terze parti (`ncdu-compare` / `ncdu-diff`) privi di UI grafica, di verifica di compatibilità tra snapshot e di gestione esplicita degli errori di accesso. |
| **dua-cli** / **gdu** | Rust (`MIT`) / Go (`MIT`) | Scansione parallela velocissima su NVMe/SSD; TUI interattiva. | Privi di interfaccia desktop grafica, privi di database storico di snapshot e privi di modalità di analisi differenziale guidata. |

---

## 3. Matrice comparativa di sintesi

| Caratteristica | QDirStat | eDirStat | Filelight | Baobab | Duc | ncdu | **DiskGarden (MVP)** |
| :--- | :---: | :---: | :---: | :---: | :---: | :---: | :---: |
| **Interfaccia Desktop Linux moderna** | Qt6 (classica) | `egui` (GPU) | Qt6 / KDE | GTK4 / GNOME | X11/CLI | Solo TUI | **Sì (Dark/Light curata)** |
| **Visualizzazione principale** | Treemap + Tree | Treemap + Plot | Sunburst | Sunburst / Treemap | Sunburst | Lista TUI | **Treemap ibrida + Lista** |
| **Classifica globale Top File / Dir** | Parziale | Sì | No | No | No (solo dir) | No | **Sì (Explore & Recover)** |
| **Ricerca e filtri istantanei** | Sì | Sì | Solo esclusioni | No | No | Filtro locale | **Sì (nome, tipo, size, stato)** |
| **Distinzione Apparente vs Allocato** | Sì | Parziale | No | No | Sì | Sì (`a` toggle) | **Sì (esplicita ovunque)** |
| **Dedup Hard Link (`st_dev`, `st_ino`)** | Sì | Sì | Parziale | Sì | Sì | Sì | **Sì (con indicazione UI)** |
| **Salvataggio Snapshot persistenti** | Cache `.gz` | File `.edst` | No | No | DB locale | Export JSON | **Sì (SQLite locale)** |
| **Diff Engine nativo tra 2 Snapshot** | **No** | **No** | **No** | **No** | **No** | Esterno (CLI) | **Sì (Core Differentiator)** |
| **Gestione scansioni parziali nel Diff** | N/A | N/A | N/A | N/A | N/A | No | **Sì (verifica compatibilità)** |
| **Qualifica spazio recuperabile (3 livelli)** | No | Solo duplicati | No | No | No | No | **Sì (Rilevato / Potenziale / Verificato)** |
| **Sicurezza (Zero delete / Zero shell)** | No (ha shell) | No (ha delete) | No (ha delete) | No (ha trash) | Sì | No (ha delete) | **Sì (100% non-distruttivo)** |

---

## 4. Differenziazione concreta di DiskGarden

Dalla verifica empirica emerge un vuoto di prodotto netto e non puramente estetico:

> **Nessuno degli analizzatori grafici Linux esistenti (QDirStat, eDirStat, Filelight, Baobab) possiede un motore di confronto temporale (Diff Engine) tra snapshot integrato nella navigazione spaziale.**  
> Anche gli strumenti che salvano lo stato su disco (QDirStat con le cache `.gz`, eDirStat con i file `.edst`, `ncdu` con i dump JSON) usano il salvataggio esclusivamente per **evitare di ripetere la scansione**, lasciando all'utente il compito di aprire due finestre e confrontare manualmente i numeri a occhio.

### I 3 pilastri differenzianti di DiskGarden

1. **Memoria Temporale Autorevole (`History` Mode + Delta Overlay in `Explore`):**
   - L'utente salva snapshot locali leggeri su SQLite (senza hashing crittografico dei contenuti).
   - Il modulo `DiffEngine` confronta due snapshot compatibili della stessa radice ($T_0 \rightarrow T_1$) calcolando:
     - **Delta netto aggregato per directory** senza doppi conteggi gerarchici (distinguendo la crescita generata dai file diretti della directory da quella propagata dalle sottodirectory).
     - **Classificazione rigorosa delle variazioni sui file:** *Aggiunto*, *Rimosso*, *Cresciuto* ($\Delta > 0$), *Ridotto* ($\Delta < 0$), *Metadati aggiornati a parità di dimensione* ($\Delta = 0, \text{mtime}_1 \neq \text{mtime}_0$, mai confuso con una crescita), e *Non confrontabile in modo affidabile* (es. sottoalbero con `PermissionDenied` in $T_0$ o $T_1$, oppure scansione parziale).
     - **Proiezione spaziale del Delta:** Quando è attivo uno snapshot di riferimento, il pannello informativo in `Explore` mostra direttamente la variazione (`+1,42 GB rispetto a 2026-10-14`) per qualsiasi cartella o file selezionato.

2. **Triage Consapevole e Qualificato dello Spazio (`Recover` Mode):**
   - A differenza dei "system cleaner" opachi (tipo BleachBit) che promettono pulizie automatiche rischiose, e a differenza di Baobab/Filelight che non offrono alcuna guida, DiskGarden presenta candidati di revisione classificati con **tre metriche distinte**:
     1. **Spazio occupato rilevato:** byte apparenti e byte effettivamente allocati (`st_blocks * 512`) sul filesystem.
     2. **Spazio potenzialmente recuperabile:** quota appartenente a pattern di cache/build/temporanei documentati e trasparenti (es. `~/.cache`, `target/debug`, `node_modules/.cache`, `__pycache__`, `.gradle/caches`, `/tmp`), con spiegazione della regola applicata.
     3. **Spazio recuperabile effettivamente verificato:** spazio fisico che verrebbe realmente liberato sul dispositivo tenendo conto degli **hard link** (`st_nlink > 1`: se un inode ha altri link fuori dalla directory esaminata, la rimozione del percorso non libera blocchi su disco) e della completezza di lettura del sottoalbero.

3. **Architettura 100% Non-Distruttiva e Trasparente sugli Errori:**
   - Le directory inaccessibili (`EACCES` / `EPERM`), i mount point non attraversati (`one_file_system`) e i link simbolici ignorati sono cittadini di prima classe nel modello dati e nella UI: non vengono mai scambiati per directory di 0 byte né silenziati durante il confronto tra snapshot.
