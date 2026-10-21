use disk_garden_app::{AppServer, AppState};
use disk_garden_core::ScanConfig;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Child, Command, Stdio};
use std::thread;
use std::time::Duration;

fn print_usage() {
    eprintln!(
        "Uso: disk-garden [PERCORSO_DIRECTORY] [OPZIONI]\n\
         \n\
         Opzioni:\n\
         \x20 --port <PORTA>       Porta locale su 127.0.0.1 (default: 0 = automatica)\n\
         \x20 --headless           Avvia solo il server IPC/UI locale senza aprire una finestra grafica\n\
         \x20 --no-initial-scan    Non avviare automaticamente la scansione all'apertura\n\
         \x20 --follow-symlinks    Segue i link simbolici nella scansione iniziale\n\
         \x20 --cross-mounts       Attraversa altri mount point nella scansione iniziale\n"
    );
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        process::exit(0);
    }

    let mut initial_dir: Option<PathBuf> = None;
    let mut port: u16 = 0;
    let mut headless = false;
    let mut no_initial_scan = false;
    let mut follow_symlinks = false;
    let mut cross_mounts = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--headless" => headless = true,
            "--no-initial-scan" => no_initial_scan = true,
            "--follow-symlinks" => follow_symlinks = true,
            "--cross-mounts" => cross_mounts = true,
            "--port" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("Errore: --port richiede un numero di porta");
                    process::exit(1);
                }
                port = args[i].parse().unwrap_or(0);
            }
            arg if !arg.starts_with('-') && initial_dir.is_none() => {
                initial_dir = Some(PathBuf::from(arg));
            }
            other => {
                eprintln!("Opzione non riconosciuta: {}", other);
                print_usage();
                process::exit(1);
            }
        }
        i += 1;
    }

    let state = AppState::new();

    if !no_initial_scan {
        // Default to the entire root disk ("/") unless the user specified a directory path on the CLI.
        let root = initial_dir.unwrap_or_else(|| PathBuf::from("/"));
        let mut cfg = ScanConfig::new(root);
        cfg.follow_symlinks = follow_symlinks;
        cfg.cross_mounts = cross_mounts;
        cfg.progress_interval_ms = 50;
        state.start_scan(cfg);
    }

    let server = match AppServer::start(port, state) {
        Ok(s) => s,
        Err(err) => {
            eprintln!("Errore avvio server locale DiskGarden: {}", err);
            process::exit(1);
        }
    };

    let url = server.url();
    println!("🌱 DiskGarden attivo su {}", url);

    if headless {
        println!("Modalità headless attiva. Premi Ctrl+C per terminare.");
        loop {
            thread::sleep(Duration::from_secs(3600));
        }
    }

    // Attempt to open a native Linux desktop window (Qt6 WebEngine or App mode window)
    if let Some(mut child) = launch_desktop_window(&url) {
        let _ = child.wait();
    } else {
        println!(
            "Finestra desktop avviata o sessione priva di display diretto.\n\
             Apri {} nel browser locale. Premi Ctrl+C per chiudere DiskGarden.",
            url
        );
        loop {
            thread::sleep(Duration::from_secs(3600));
        }
    }
}

fn launch_desktop_window(url: &str) -> Option<Child> {
    let has_display = env::var_os("WAYLAND_DISPLAY").is_some() || env::var_os("DISPLAY").is_some();
    if !has_display {
        return None;
    }

    // 1. Try native Qt6 WebEngine window via `/usr/bin/qml6`
    if Path::new("/usr/bin/qml6").exists() {
        let qml_path = env::temp_dir().join(format!("diskgarden_window_{}.qml", process::id()));
        let qml_source = format!(
            "import QtQuick\n\
             import QtQuick.Window\n\
             import QtWebEngine\n\
             Window {{\n\
                 width: 1320\n\
                 height: 840\n\
                 visible: true\n\
                 title: \"DiskGarden\"\n\
                 WebEngineView {{\n\
                     anchors.fill: parent\n\
                     url: \"{}\"\n\
                 }}\n\
             }}\n",
            url
        );
        if fs::write(&qml_path, qml_source).is_ok() {
            if let Ok(mut child) = Command::new("/usr/bin/qml6")
                .arg(&qml_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                // Give qml6 350ms to verify it didn't exit immediately due to missing plugin
                thread::sleep(Duration::from_millis(350));
                if let Ok(None) = child.try_wait() {
                    return Some(child);
                }
            }
        }
    }

    // 2. Fallback to Chromium/Brave native `--app=...` desktop window
    for bin in [
        "/usr/bin/brave-browser-stable",
        "/usr/bin/brave-browser",
        "/usr/bin/chromium",
        "/usr/bin/google-chrome",
    ] {
        if Path::new(bin).exists() {
            if let Ok(mut child) = Command::new(bin)
                .arg(format!("--app={}", url))
                .arg("--window-size=1320,840")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                thread::sleep(Duration::from_millis(250));
                if let Ok(None) = child.try_wait() {
                    return Some(child);
                }
            }
        }
    }

    // 3. Fallback to xdg-open
    let _ = Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    None
}
