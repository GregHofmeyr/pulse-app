//! What game runs now (#380), from what a [`Source`] reads: Steam's running app, named from its manifest in one of
//! Steam's libraries, else a running process matching one of the games added by hand. Pure apart from the source and
//! the files it points at, so it is tested with a fake source and a temp dir.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::vdf;

/// A game added by hand in Settings: the exe picked, and the name to show. The path never leaves this computer.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ManualGame {
    pub name: String,
    pub exe: String,
}

/// A game seen running.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Detected {
    pub name: String,
    /// Steam's app id, when Steam runs it.
    pub steam_app_id: Option<u32>,
}

/// Where detection reads from (the real one: the Windows registry and process list, `win.rs`).
pub trait Source: Send {
    /// Steam's `RunningAppID` (0 or `None`: no Steam game).
    fn running_app_id(&mut self) -> Option<u32>;
    /// Steam's install folder (`SteamPath`), whose `steamapps/libraryfolders.vdf` lists every library.
    fn steam_path(&mut self) -> Option<PathBuf>;
    /// Every running process's image path where Windows gives it, else its exe name.
    fn processes(&mut self) -> Vec<String>;
}

/// Steam game names by app id, read from the manifests and kept: a game's name is read once. The library list is
/// read again only when an app id isn't found in any library already known (a new drive, a new install).
#[derive(Default)]
pub struct SteamNames {
    libraries: Option<Vec<PathBuf>>,
    names: HashMap<u32, Option<String>>,
}

impl SteamNames {
    /// Steam app `id`'s name, from the manifests under `steam` (its install folder).
    pub fn name(&mut self, steam: &Path, id: u32) -> Option<String> {
        if let Some(n) = self.names.get(&id) {
            return n.clone();
        }
        let mut found = self.libraries.as_deref().and_then(|l| find(l, id));
        if found.is_none() {
            let libs = libraries(steam);
            found = find(&libs, id);
            self.libraries = Some(libs);
        }
        // bounded: the games played on this computer
        if self.names.len() >= 512 {
            self.names.clear();
        }
        self.names.insert(id, found.clone());
        found
    }
}

/// Steam's own folder and every library its `libraryfolders.vdf` lists, without repeats.
fn libraries(steam: &Path) -> Vec<PathBuf> {
    let mut out = vec![steam.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(steam.join("steamapps").join("libraryfolders.vdf")) {
        for p in vdf::library_paths(&text) {
            if !out.iter().any(|o| same_path(o, &p)) {
                out.push(p);
            }
        }
    }
    out
}

fn find(libraries: &[PathBuf], id: u32) -> Option<String> {
    libraries.iter().find_map(|lib| {
        let text =
            std::fs::read_to_string(lib.join("steamapps").join(format!("appmanifest_{id}.acf")))
                .ok()?;
        vdf::manifest_name(&text)
    })
}

/// A path as Windows compares it: no case, either slash.
fn norm(p: &str) -> String {
    p.trim().replace('/', "\\").to_lowercase()
}

fn same_path(a: &Path, b: &Path) -> bool {
    norm(&a.to_string_lossy()) == norm(&b.to_string_lossy())
}

/// The last part of a path (either slash).
fn file_name(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

/// The first of `games` running among `procs`: by full path, or by exe name alone for a process whose path Windows
/// wouldn't give (only its name is known then).
pub fn match_exe<'a>(procs: &[String], games: &'a [ManualGame]) -> Option<&'a ManualGame> {
    let procs: Vec<String> = procs.iter().map(|p| norm(p)).collect();
    games.iter().find(|g| {
        let want = norm(&g.exe);
        if want.is_empty() {
            return false;
        }
        let want_name = file_name(&want);
        procs
            .iter()
            .any(|p| *p == want || (!p.contains('\\') && p == want_name))
    })
}

/// What game runs now: Steam's running app when Steam names it, else the first hand-added game running. The process
/// list is read only when there are hand-added games.
pub fn detect(
    source: &mut dyn Source,
    names: &mut SteamNames,
    games: &[ManualGame],
) -> Option<Detected> {
    if let Some(id) = source.running_app_id().filter(|id| *id != 0)
        && let Some(steam) = source.steam_path()
        && let Some(name) = names.name(&steam, id)
    {
        return Some(Detected {
            name,
            steam_app_id: Some(id),
        });
    }
    if games.is_empty() {
        return None;
    }
    match_exe(&source.processes(), games).map(|g| Detected {
        name: g.name.clone(),
        steam_app_id: None,
    })
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A source that says what it's told, counting process reads.
    #[derive(Default)]
    pub struct Fake {
        pub app_id: Option<u32>,
        pub steam: Option<PathBuf>,
        pub procs: Vec<String>,
        pub proc_reads: usize,
    }

    impl Source for Fake {
        fn running_app_id(&mut self) -> Option<u32> {
            self.app_id
        }
        fn steam_path(&mut self) -> Option<PathBuf> {
            self.steam.clone()
        }
        fn processes(&mut self) -> Vec<String> {
            self.proc_reads += 1;
            self.procs.clone()
        }
    }

    fn game(name: &str, exe: &str) -> ManualGame {
        ManualGame {
            name: name.into(),
            exe: exe.into(),
        }
    }

    fn manifest(lib: &Path, id: u32, name: &str) {
        let apps = lib.join("steamapps");
        std::fs::create_dir_all(&apps).unwrap();
        std::fs::write(
            apps.join(format!("appmanifest_{id}.acf")),
            format!("\"AppState\"\n{{\n\t\"appid\"\t\"{id}\"\n\t\"name\"\t\"{name}\"\n}}\n"),
        )
        .unwrap();
    }

    fn library_list(steam: &Path, libs: &[&Path]) {
        let apps = steam.join("steamapps");
        std::fs::create_dir_all(&apps).unwrap();
        let mut text = String::from("\"libraryfolders\"\n{\n");
        for (i, l) in libs.iter().enumerate() {
            let p = l.to_string_lossy().replace('\\', "\\\\");
            text.push_str(&format!("\t\"{i}\"\n\t{{\n\t\t\"path\"\t\"{p}\"\n\t}}\n"));
        }
        text.push('}');
        std::fs::write(apps.join("libraryfolders.vdf"), text).unwrap();
    }

    #[test]
    fn exes_match_by_path_whatever_the_case_and_slashes() {
        let games = [
            game("Old Game", r"D:\Games\Old\old.exe"),
            game("Other", "E:/x/other.exe"),
        ];
        let procs = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            match_exe(
                &procs(&[r"C:\Windows\explorer.exe", r"d:/games/OLD/Old.EXE"]),
                &games
            )
            .map(|g| g.name.as_str()),
            Some("Old Game")
        );
        assert_eq!(
            match_exe(&procs(&[r"e:\X\OTHER.exe"]), &games).map(|g| g.name.as_str()),
            Some("Other")
        );
        // the same exe name somewhere else isn't the game
        assert_eq!(match_exe(&procs(&[r"C:\Elsewhere\old.exe"]), &games), None);
        // a process whose path Windows wouldn't give: its name alone
        assert_eq!(
            match_exe(&procs(&["old.exe"]), &games).map(|g| g.name.as_str()),
            Some("Old Game")
        );
        assert_eq!(match_exe(&procs(&[]), &games), None);
        assert_eq!(match_exe(&procs(&[""]), &[game("Blank", "")]), None);
    }

    #[test]
    fn a_steam_game_is_named_from_whichever_library_has_it() {
        let dir = tempfile::tempdir().unwrap();
        let steam = dir.path().join("Steam");
        let second = dir.path().join("Other drive").join("SteamLibrary");
        manifest(&steam, 228980, "Runtime");
        manifest(&second, 570, "Space Game");
        library_list(&steam, &[&steam, &second]);
        let mut src = Fake {
            app_id: Some(570),
            steam: Some(steam.clone()),
            ..Fake::default()
        };
        let mut names = SteamNames::default();
        assert_eq!(
            detect(&mut src, &mut names, &[]),
            Some(Detected {
                name: "Space Game".into(),
                steam_app_id: Some(570)
            })
        );
        // read once: the manifest going away changes nothing
        std::fs::remove_file(second.join("steamapps").join("appmanifest_570.acf")).unwrap();
        assert_eq!(
            detect(&mut src, &mut names, &[]).map(|d| d.name),
            Some("Space Game".into())
        );
        assert_eq!(src.proc_reads, 0, "no hand-added games: no process list");

        // a library added since: found by reading the list again
        let third = dir.path().join("Third");
        manifest(&third, 440, "Hat Game");
        library_list(&steam, &[&steam, &second, &third]);
        src.app_id = Some(440);
        assert_eq!(
            detect(&mut src, &mut names, &[]).map(|d| d.name),
            Some("Hat Game".into())
        );
    }

    #[test]
    fn no_steam_game_falls_back_to_hand_added_ones() {
        let dir = tempfile::tempdir().unwrap();
        let games = [game("Old Game", r"D:\Games\old.exe")];
        let mut src = Fake {
            app_id: Some(0),
            steam: Some(dir.path().to_path_buf()),
            procs: vec![r"D:\Games\old.exe".into()],
            ..Fake::default()
        };
        let mut names = SteamNames::default();
        assert_eq!(
            detect(&mut src, &mut names, &games),
            Some(Detected {
                name: "Old Game".into(),
                steam_app_id: None
            })
        );
        // a Steam id with no manifest anywhere (a non-Steam shortcut): the hand-added game still counts
        src.app_id = Some(0x8000_0001);
        assert_eq!(
            detect(&mut src, &mut names, &games).map(|d| d.steam_app_id),
            Some(None)
        );
        // no Steam at all, nothing running
        src.steam = None;
        src.procs.clear();
        assert_eq!(detect(&mut src, &mut names, &games), None);
    }
}
