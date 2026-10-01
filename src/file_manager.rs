use std::{path::Path, process::Command, thread};

/// Reveal and select the original photo without blocking the UI.
pub fn open_photo_folder(photo_path: &Path) {
    let photo_path = photo_path.to_path_buf();
    thread::spawn(move || {
        let result = (|| -> Result<(), String> {
            let absolute = std::path::absolute(&photo_path).map_err(|e| e.to_string())?;
            if !absolute.is_file() {
                return Err(format!("Photo introuvable : {}", absolute.display()));
            }
            #[cfg(target_os = "windows")]
            let mut command = {
                let mut command = Command::new("explorer.exe");
                let mut argument = std::ffi::OsString::from("/select,");
                argument.push(&absolute);
                command.arg(argument);
                command
            };
            #[cfg(target_os = "macos")]
            let mut command = {
                let mut command = Command::new("open");
                command.arg("-R").arg(&absolute);
                command
            };
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            let mut command = {
                let uri = url::Url::from_file_path(&absolute)
                    .map_err(|_| "Impossible de convertir le chemin de la photo en URI.")?;
                let mut command = Command::new("gdbus");
                // Escape apostrophes for the GVariant string array, not for a shell.
                let items = format!("['{}']", uri.as_str().replace('\'', "%27"));
                command.args([
                    "call", "--session", "--timeout", "10",
                    "--dest", "org.freedesktop.FileManager1",
                    "--object-path", "/org/freedesktop/FileManager1",
                    "--method", "org.freedesktop.FileManager1.ShowItems",
                    &items, "",
                ]);
                command
            };

            let output = command.output()
                .map_err(|e| format!("Impossible de lancer l'explorateur : {e}"))?;
            if !output.status.success() {
                return Err(format!("Impossible de sélectionner la photo : {}",
                    String::from_utf8_lossy(&output.stderr).trim()));
            }
            Ok(())
        })();
        if let Err(error) = result {
            rfd::MessageDialog::new()
                .set_title("Impossible d'ouvrir le dossier")
                .set_description(&error)
                .set_level(rfd::MessageLevel::Error)
                .show();
        }
    });
}
