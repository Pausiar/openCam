//! Emparejamientos guardados del escritorio: id propio y clave a largo plazo por móvil.
//!
//! Formato (texto, una entrada por línea):
//! ```text
//! client_id <hex de 16 bytes>
//! phone <phone_id> <hex de 32 bytes>
//! ```
//! En Unix el archivo se crea con permisos 0600: quien lo lea puede suplantar a este PC.

use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use opencam_core::secure::{from_hex, random_bytes, to_hex};

pub struct PairingStore {
    path: Option<PathBuf>,
    client_id: String,
    keys: Vec<(String, [u8; 32])>,
}

impl PairingStore {
    /// Sin archivo: útil en tests o si no hay carpeta de configuración.
    pub fn in_memory() -> Self {
        Self {
            path: None,
            client_id: new_id(),
            keys: Vec::new(),
        }
    }

    /// Carga el archivo por defecto; si no se puede, sigue en memoria y lo avisa.
    pub fn load_default() -> Self {
        match default_path() {
            Some(p) => Self::load(&p).unwrap_or_else(|e| {
                eprintln!(
                    "Aviso: no se pudo leer {} ({e}); los emparejamientos no se guardarán",
                    p.display()
                );
                Self::in_memory()
            }),
            None => Self::in_memory(),
        }
    }

    /// Carga `path` (o empieza vacío si no existe) y guarda los cambios ahí.
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut s = Self {
            path: Some(path.to_path_buf()),
            client_id: String::new(),
            keys: Vec::new(),
        };
        match fs::read_to_string(path) {
            Ok(text) => {
                for line in text.lines() {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    match parts.as_slice() {
                        ["client_id", id] if is_id(id) => s.client_id = id.to_string(),
                        ["phone", id, key] if is_id(id) => {
                            if let Some(k) = from_hex(key).and_then(|k| k.try_into().ok()) {
                                s.keys.push((id.to_string(), k));
                            }
                        }
                        _ => {} // líneas desconocidas o dañadas: se ignoran
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        if s.client_id.is_empty() {
            s.client_id = new_id();
            s.save()?;
        }
        Ok(s)
    }

    pub fn client_id(&self) -> &str {
        &self.client_id
    }

    pub fn key_for(&self, phone_id: &str) -> Option<[u8; 32]> {
        self.keys
            .iter()
            .find(|(id, _)| id == phone_id)
            .map(|(_, k)| *k)
    }

    pub fn remember(&mut self, phone_id: &str, key: [u8; 32]) -> io::Result<()> {
        if !is_id(phone_id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "phone_id inválido",
            ));
        }
        self.keys.retain(|(id, _)| id != phone_id);
        self.keys.push((phone_id.to_string(), key));
        self.save()
    }

    #[allow(dead_code)] // para la interfaz («olvidar este móvil»)
    pub fn forget(&mut self, phone_id: &str) -> io::Result<()> {
        self.keys.retain(|(id, _)| id != phone_id);
        self.save()
    }

    fn save(&self) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut text = format!("client_id {}\n", self.client_id);
        for (id, k) in &self.keys {
            text.push_str(&format!("phone {id} {}\n", to_hex(k)));
        }
        // Escritura atómica: archivo temporal privado + rename.
        let tmp = path.with_extension("tmp");
        {
            let mut f = private_file(&tmp)?;
            f.write_all(text.as_bytes())?;
            f.sync_all()?;
        }
        fs::rename(&tmp, path)
    }
}

impl fmt::Debug for PairingStore {
    // No imprime las claves.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PairingStore")
            .field("path", &self.path)
            .field("client_id", &self.client_id)
            .field("phones", &self.keys.len())
            .finish()
    }
}

/// Ids hex de 4 a 64 caracteres: así nunca rompen el formato del archivo ni el JSON.
pub fn is_id(s: &str) -> bool {
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn new_id() -> String {
    to_hex(&random_bytes::<16>().expect("el sistema no da números aleatorios"))
}

#[cfg(unix)]
fn private_file(p: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let _ = fs::remove_file(p);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(p)
}

#[cfg(not(unix))]
fn private_file(p: &Path) -> io::Result<fs::File> {
    // En Windows %APPDATA% ya es privado del usuario.
    fs::File::create(p)
}

/// `$XDG_CONFIG_HOME/opencam/pairings` (Linux), `~/Library/Application Support/opencam/…`
/// (macOS) o `%APPDATA%\opencam\pairings` (Windows).
pub fn default_path() -> Option<PathBuf> {
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        env("APPDATA")
    } else if cfg!(target_os = "macos") {
        env("HOME").map(|h| h.join("Library/Application Support"))
    } else {
        env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
    }?;
    Some(base.join("opencam").join("pairings"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "opencam-test-{name}-{}",
            to_hex(&random_bytes::<6>().unwrap())
        ));
        dir.join("pairings")
    }

    #[test]
    fn persists_ids_and_keys() {
        let p = tmp_path("persist");
        let mut s = PairingStore::load(&p).unwrap();
        let id = s.client_id().to_string();
        assert!(is_id(&id));
        s.remember("abcd1234", [9; 32]).unwrap();
        s.remember("abcd1234", [8; 32]).unwrap(); // sustituye
        s.remember("00ff00ff", [7; 32]).unwrap();

        let mut s2 = PairingStore::load(&p).unwrap();
        assert_eq!(s2.client_id(), id);
        assert_eq!(s2.key_for("abcd1234"), Some([8; 32]));
        assert_eq!(s2.key_for("00ff00ff"), Some([7; 32]));
        assert_eq!(s2.key_for("ffff"), None);
        s2.forget("abcd1234").unwrap();
        assert_eq!(PairingStore::load(&p).unwrap().key_for("abcd1234"), None);
        // Debug no muestra claves.
        assert!(!format!("{s2:?}").contains("0707"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&p).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "el archivo no debe ser legible por otros");
        }
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn ignores_garbage_and_rejects_bad_ids() {
        let p = tmp_path("garbage");
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(
            &p,
            "client_id 0011223344\nphone zz 00\nphone abcd 1234\nbasura\nphone beef ".to_string()
                + &"11".repeat(32)
                + "\n",
        )
        .unwrap();
        let mut s = PairingStore::load(&p).unwrap();
        assert_eq!(s.client_id(), "0011223344");
        assert_eq!(s.key_for("abcd"), None); // clave corta
        assert_eq!(s.key_for("beef"), Some([0x11; 32]));
        assert!(s.remember("no es id\nphone", [0; 32]).is_err());
        let _ = fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn ids() {
        assert!(is_id("abcd"));
        assert!(!is_id("abc"));
        assert!(!is_id("abcg"));
        assert!(!is_id(&"a".repeat(65)));
    }
}
