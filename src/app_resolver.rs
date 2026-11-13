use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const DEFAULT_DISCORD_DETECTABLE_URL: &str =
    "https://discord.com/api/v9/applications/detectable";
pub const CACHE_TTL_SECONDS: u64 = 24 * 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectableApp {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub executables: Vec<DetectableExecutable>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectableExecutable {
    pub name: String,
    #[serde(default)]
    pub os: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedAppCandidate {
    pub discord_app_id: String,
    pub discord_app_name: String,
    pub score: u32,
}

pub const GENERIC_EXE_BLACKLIST: &[&str] = &[
    "app.exe",
    "main.exe",
    "client.exe",
    "launcher.exe",
    "game.exe",
    "start.exe",
    "setup.exe",
    "install.exe",
    "uninstall.exe",
    "unins000.exe",
    "update.exe",
    "updater.exe",
    "crashreporter.exe",
    "python.exe",
    "pythonw.exe",
    "javaw.exe",
    "java.exe",
    "node.exe",
    "electron.exe",
    "cmd.exe",
    "powershell.exe",
    "conhost.exe",
];

pub fn is_generic_executable(exe_name: &str) -> bool {
    let file_name = exe_name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(exe_name)
        .trim();
    let lower = file_name.to_ascii_lowercase();

    GENERIC_EXE_BLACKLIST.iter().any(|&item| {
        item.eq_ignore_ascii_case(&lower)
            || item
                .strip_suffix(".exe")
                .is_some_and(|base| base.eq_ignore_ascii_case(&lower))
    })
}

pub fn normalize_executable_name(name: &str) -> String {
    let lower = name.replace('\\', "/").to_ascii_lowercase();
    let has_exe = lower.ends_with(".exe");
    let stem = if has_exe {
        &lower[..lower.len() - 4]
    } else {
        &lower
    };

    let suffixes = ["_win64", "-win64", "_x64", "-x64", "x64", "64"];
    for suffix in suffixes {
        if let Some(stripped) = stem.strip_suffix(suffix)
            && !stripped.is_empty()
            && !stripped.ends_with('/')
        {
            return if has_exe {
                format!("{}.exe", stripped)
            } else {
                stripped.to_string()
            };
        }
    }

    lower
}

pub fn extract_app_keywords(app_name: &str) -> Vec<String> {
    let words: Vec<String> = app_name
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.len() >= 3)
        .map(|w| w.to_ascii_lowercase())
        .collect();

    if words.is_empty() {
        let trimmed = app_name.trim().to_ascii_lowercase();
        if !trimmed.is_empty() {
            vec![trimmed]
        } else {
            vec![]
        }
    } else {
        words
    }
}

pub fn get_parent_folder_names(path: &str) -> Vec<String> {
    let normalized = path.replace('\\', "/");
    let parts: Vec<&str> = normalized.split('/').filter(|p| !p.is_empty()).collect();
    let mut folders = Vec::new();

    if parts.len() >= 2 {
        let immediate = parts[parts.len() - 2].to_ascii_lowercase();
        folders.push(immediate.clone());

        let common_subfolders = [
            "bin", "x64", "win64", "win32", "shipping", "release", "app",
        ];
        if common_subfolders.contains(&immediate.as_str()) && parts.len() >= 3 {
            folders.push(parts[parts.len() - 3].to_ascii_lowercase());
        }
    }

    folders
}

pub fn score_candidate(full_exe_path: &str, app: &DetectableApp) -> u32 {
    let norm_full_path = normalize_executable_name(full_exe_path);
    let norm_base_exe = norm_full_path
        .rsplit('/')
        .next()
        .unwrap_or(&norm_full_path);

    if is_generic_executable(norm_base_exe) || is_generic_executable(full_exe_path) {
        return 0;
    }

    let stem = norm_base_exe.strip_suffix(".exe").unwrap_or(norm_base_exe);
    let parent_folders = get_parent_folder_names(full_exe_path);
    let app_keywords = extract_app_keywords(&app.name);

    let mut best_score = 0;

    for exec in &app.executables {
        let os = exec.os.trim();
        if !os.is_empty() && !os.eq_ignore_ascii_case("win32") {
            continue;
        }

        let norm_exec = normalize_executable_name(&exec.name);
        let norm_exec_base = norm_exec.rsplit('/').next().unwrap_or(&norm_exec);

        // Score 100 (Very High): Folder + Exe matches executable entry (e.g., blender/blender.exe)
        if norm_exec.contains('/') {
            let matched_100 = norm_full_path == norm_exec
                || norm_full_path.ends_with(&format!("/{}", norm_exec));
            if matched_100 {
                return 100;
            }
        }

        // Check if normalized base exe name matches exactly
        let base_matches = norm_base_exe.eq_ignore_ascii_case(norm_exec_base)
            || stem.eq_ignore_ascii_case(
                norm_exec_base
                    .strip_suffix(".exe")
                    .unwrap_or(norm_exec_base),
            );

        if !base_matches {
            continue;
        }

        // Score 80 (High): Normalized exe name matches AND parent folder name contains keywords from app.name
        let folder_keyword_matched = parent_folders
            .iter()
            .any(|folder| app_keywords.iter().any(|kw| folder.contains(kw)));

        if folder_keyword_matched {
            best_score = best_score.max(80);
            continue;
        }

        // Score 60 (Fair): Normalized base exe name matches exactly, provided base name without extension has length >= 5 chars
        if stem.len() >= 5 {
            best_score = best_score.max(60);
        }
    }

    best_score
}

pub fn resolve_best_candidate(
    full_exe_path: &str,
    detectable_apps: &[DetectableApp],
) -> Option<ResolvedAppCandidate> {
    let mut best_candidate: Option<ResolvedAppCandidate> = None;

    for app in detectable_apps {
        let score = score_candidate(full_exe_path, app);
        if score >= 60 {
            let replace = match &best_candidate {
                None => true,
                Some(current) => score > current.score,
            };

            if replace {
                best_candidate = Some(ResolvedAppCandidate {
                    discord_app_id: app.id.clone(),
                    discord_app_name: app.name.clone(),
                    score,
                });

                if score == 100 {
                    break;
                }
            }
        }
    }

    best_candidate
}

pub fn cache_path() -> PathBuf {
    if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
        PathBuf::from(local_app_data)
            .join("larplife")
            .join("detectable_cache.json")
    } else if let Ok(app_data) = std::env::var("APPDATA") {
        PathBuf::from(app_data)
            .join("larplife")
            .join("detectable_cache.json")
    } else {
        std::env::temp_dir()
            .join("larplife")
            .join("detectable_cache.json")
    }
}

pub fn is_cache_fresh(cache_file: &Path, now: SystemTime) -> bool {
    if let Ok(metadata) = fs::metadata(cache_file) {
        if let Ok(modified) = metadata.modified() {
            match now.duration_since(modified) {
                Ok(elapsed) => elapsed.as_secs() < CACHE_TTL_SECONDS,
                Err(_) => true,
            }
        } else {
            false
        }
    } else {
        false
    }
}

pub fn read_cache_file(cache_file: &Path) -> Result<Vec<DetectableApp>, String> {
    let content = fs::read_to_string(cache_file).map_err(|e| e.to_string())?;
    serde_json::from_str::<Vec<DetectableApp>>(&content).map_err(|e| e.to_string())
}

pub fn write_cache_file(cache_file: &Path, apps: &[DetectableApp]) -> Result<(), String> {
    if let Some(parent) = cache_file.parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = fs::create_dir_all(parent);
    }

    let json = serde_json::to_string(apps).map_err(|e| e.to_string())?;
    fs::write(cache_file, json).map_err(|e| e.to_string())
}

pub fn fetch_from_network(url: &str) -> Result<Vec<DetectableApp>, String> {
    let resp = ureq::get(url)
        .set("User-Agent", "Larplife/0.3.0")
        .timeout(Duration::from_secs(25))
        .call()
        .map_err(|e| e.to_string())?;

    let reader = resp.into_reader();
    serde_json::from_reader::<_, Vec<DetectableApp>>(reader).map_err(|e| e.to_string())
}

pub fn fetch_detectable_apps_at_time(
    url: &str,
    cache_file: &Path,
    force_refresh: bool,
    now: SystemTime,
) -> Result<Vec<DetectableApp>, String> {
    if !force_refresh
        && is_cache_fresh(cache_file, now)
        && let Ok(cached_apps) = read_cache_file(cache_file)
    {
        return Ok(cached_apps);
    }

    match fetch_from_network(url) {
        Ok(apps) => {
            let _ = write_cache_file(cache_file, &apps);
            Ok(apps)
        }
        Err(net_err) => {
            if cache_file.exists() {
                match read_cache_file(cache_file) {
                    Ok(stale_apps) => return Ok(stale_apps),
                    Err(cache_err) => {
                        return Err(format!(
                            "Network request failed: {}; corrupted cache: {}",
                            net_err, cache_err
                        ));
                    }
                }
            }
            Err(format!("Network request failed: {}", net_err))
        }
    }
}

pub fn fetch_detectable_apps_from_url(
    url: &str,
    cache_file: &Path,
    force_refresh: bool,
) -> Result<Vec<DetectableApp>, String> {
    fetch_detectable_apps_at_time(url, cache_file, force_refresh, SystemTime::now())
}

pub fn fetch_detectable_apps(
    cache_file: &Path,
    force_refresh: bool,
) -> Result<Vec<DetectableApp>, String> {
    fetch_detectable_apps_from_url(DEFAULT_DISCORD_DETECTABLE_URL, cache_file, force_refresh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    #[test]
    fn blacklist_rejects_generic_executables_and_allows_specific_names() {
        assert!(is_generic_executable("app.exe"));
        assert!(is_generic_executable("Main.exe"));
        assert!(is_generic_executable("CLIENT.EXE"));
        assert!(is_generic_executable("launcher.exe"));
        assert!(is_generic_executable("game.exe"));
        assert!(is_generic_executable("python.exe"));
        assert!(is_generic_executable("pythonw.exe"));
        assert!(is_generic_executable("node.exe"));
        assert!(is_generic_executable("cmd.exe"));
        assert!(is_generic_executable("powershell.exe"));
        assert!(is_generic_executable("conhost.exe"));
        assert!(is_generic_executable("C:\\Program Files\\Game\\launcher.exe"));
        assert!(is_generic_executable("D:/Tools/updater.exe"));

        assert!(!is_generic_executable("blender.exe"));
        assert!(!is_generic_executable("photoshop.exe"));
        assert!(!is_generic_executable("code.exe"));
        assert!(!is_generic_executable("kambingsim.exe"));
        assert!(!is_generic_executable("gta5.exe"));
    }

    #[test]
    fn normalization_removes_architecture_suffixes_and_handles_slashes() {
        assert_eq!(
            normalize_executable_name("Doom_x64.exe"),
            "doom.exe"
        );
        assert_eq!(
            normalize_executable_name("Subnautica64.exe"),
            "subnautica.exe"
        );
        assert_eq!(
            normalize_executable_name("Starbound_win64.exe"),
            "starbound.exe"
        );
        assert_eq!(
            normalize_executable_name("Game-win64.exe"),
            "game.exe"
        );
        assert_eq!(
            normalize_executable_name("Appx64.exe"),
            "app.exe"
        );
        assert_eq!(
            normalize_executable_name(r"C:\Games\Doom\Doom_x64.exe"),
            "c:/games/doom/doom.exe"
        );
        assert_eq!(
            normalize_executable_name("Blender/Blender.EXE"),
            "blender/blender.exe"
        );
        assert_eq!(
            normalize_executable_name("64.exe"),
            "64.exe"
        );
        assert_eq!(
            normalize_executable_name("tool.exe"),
            "tool.exe"
        );
    }

    #[test]
    fn scoring_prefers_folder_plus_exe_over_plain_exe() {
        let app_folder_match = DetectableApp {
            id: "111111111111111111".to_string(),
            name: "Blender 3D".to_string(),
            executables: vec![DetectableExecutable {
                name: "blender/blender.exe".to_string(),
                os: "win32".to_string(),
            }],
        };

        let app_plain_match = DetectableApp {
            id: "222222222222222222".to_string(),
            name: "Generic 3D Viewer".to_string(),
            executables: vec![DetectableExecutable {
                name: "blender.exe".to_string(),
                os: "win32".to_string(),
            }],
        };

        let path = r"C:\Program Files\Blender\blender.exe";
        let score_100 = score_candidate(path, &app_folder_match);
        let score_60 = score_candidate(path, &app_plain_match);

        assert_eq!(score_100, 100);
        assert_eq!(score_60, 60);

        let candidate = resolve_best_candidate(path, &[app_plain_match, app_folder_match])
            .expect("must resolve best candidate");
        assert_eq!(candidate.discord_app_id, "111111111111111111");
        assert_eq!(candidate.score, 100);
    }

    #[test]
    fn scoring_folder_keyword_tier_eighty_matches() {
        let gta = DetectableApp {
            id: "333333333333333333".to_string(),
            name: "Grand Theft Auto V".to_string(),
            executables: vec![DetectableExecutable {
                name: "gta5.exe".to_string(),
                os: "win32".to_string(),
            }],
        };

        let path_matching_folder = r"D:\Games\Grand Theft Auto V\gta5.exe";
        assert_eq!(score_candidate(path_matching_folder, &gta), 80);

        let path_unrelated_folder = r"D:\Games\RandomFolder\gta5.exe";
        assert_eq!(score_candidate(path_unrelated_folder, &gta), 0);
    }

    #[test]
    fn scoring_rejects_short_executable_names_under_five_chars_for_tier_sixty() {
        let short_app = DetectableApp {
            id: "444444444444444444".to_string(),
            name: "Arma Tactics".to_string(),
            executables: vec![DetectableExecutable {
                name: "arma.exe".to_string(),
                os: "win32".to_string(),
            }],
        };

        let path_unrelated_folder = r"C:\Games\Strategy\arma.exe";
        // "arma" len is 4 (< 5) and parent "Strategy" does not contain keywords from "Arma Tactics"
        assert_eq!(score_candidate(path_unrelated_folder, &short_app), 0);

        let long_app = DetectableApp {
            id: "555555555555555555".to_string(),
            name: "Subnautica Deep".to_string(),
            executables: vec![DetectableExecutable {
                name: "subnautica.exe".to_string(),
                os: "win32".to_string(),
            }],
        };

        let path_long = r"C:\Games\Unrelated\subnautica.exe";
        assert_eq!(score_candidate(path_long, &long_app), 60);
    }

    #[test]
    fn false_positive_prevention_kambing_kuring_does_not_match_kambingsim() {
        let goat_sim = DetectableApp {
            id: "666666666666666666".to_string(),
            name: "Kambing Simulator".to_string(),
            executables: vec![DetectableExecutable {
                name: "kambingsim.exe".to_string(),
                os: "win32".to_string(),
            }],
        };

        let false_positive_path = r"C:\Games\Kambing\KambingKuring.exe";
        assert_eq!(score_candidate(false_positive_path, &goat_sim), 0);
        assert!(resolve_best_candidate(false_positive_path, &[goat_sim.clone()]).is_none());

        let true_positive_path = r"C:\Games\Kambing\kambingsim.exe";
        assert_eq!(score_candidate(true_positive_path, &goat_sim), 80);
        let candidate = resolve_best_candidate(true_positive_path, &[goat_sim])
            .expect("must resolve genuine candidate");
        assert_eq!(candidate.discord_app_id, "666666666666666666");
    }

    #[test]
    fn generic_executables_always_score_zero() {
        let app = DetectableApp {
            id: "777777777777777777".to_string(),
            name: "Launcher Suite".to_string(),
            executables: vec![DetectableExecutable {
                name: "launcher/launcher.exe".to_string(),
                os: "win32".to_string(),
            }],
        };

        let path = r"C:\Games\launcher\launcher.exe";
        assert_eq!(score_candidate(path, &app), 0);
        assert!(resolve_best_candidate(path, &[app]).is_none());
    }

    #[test]
    fn os_filter_ignores_non_windows_executables() {
        let app = DetectableApp {
            id: "888888888888888888".to_string(),
            name: "Linux Only App".to_string(),
            executables: vec![DetectableExecutable {
                name: "game.exe".to_string(),
                os: "linux".to_string(),
            }],
        };

        let path = r"C:\Games\game.exe";
        assert_eq!(score_candidate(path, &app), 0);
    }

    #[test]
    fn cache_fresh_loads_from_disk_without_network() {
        let temp_dir = std::env::temp_dir().join("larplife_test_cache_fresh");
        let _ = fs::create_dir_all(&temp_dir);
        let cache_file = temp_dir.join("detectable_cache.json");

        let apps = vec![DetectableApp {
            id: "999999999999999999".to_string(),
            name: "Cached App".to_string(),
            executables: vec![DetectableExecutable {
                name: "cached.exe".to_string(),
                os: "win32".to_string(),
            }],
        }];

        let json = serde_json::to_string(&apps).unwrap();
        fs::write(&cache_file, json).unwrap();

        let loaded = fetch_detectable_apps_from_url(
            "http://127.0.0.1:0/never_called",
            &cache_file,
            false,
        )
        .expect("must load from fresh cache");

        assert_eq!(loaded, apps);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn cache_stale_while_error_loads_expired_cache_on_network_failure() {
        let temp_dir = std::env::temp_dir().join("larplife_test_cache_stale");
        let _ = fs::create_dir_all(&temp_dir);
        let cache_file = temp_dir.join("detectable_cache.json");

        let apps = vec![DetectableApp {
            id: "123123123123123123".to_string(),
            name: "Stale App".to_string(),
            executables: vec![DetectableExecutable {
                name: "stale.exe".to_string(),
                os: "win32".to_string(),
            }],
        }];

        let json = serde_json::to_string(&apps).unwrap();
        fs::write(&cache_file, json).unwrap();

        let future_time = SystemTime::now() + Duration::from_secs(48 * 3600);

        let loaded = fetch_detectable_apps_at_time(
            "http://127.0.0.1:0/unreachable",
            &cache_file,
            false,
            future_time,
        )
        .expect("must fallback to stale cache on network failure");

        assert_eq!(loaded, apps);
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn corrupted_cache_handled_gracefully_without_panic() {
        let temp_dir = std::env::temp_dir().join("larplife_test_cache_corrupt");
        let _ = fs::create_dir_all(&temp_dir);
        let cache_file = temp_dir.join("detectable_cache.json");

        fs::write(&cache_file, "INVALID JSON DATA {{{").unwrap();

        let result = fetch_detectable_apps_from_url(
            "http://127.0.0.1:0/unreachable",
            &cache_file,
            false,
        );

        assert!(result.is_err(), "must fail gracefully without panic");
        let _ = fs::remove_dir_all(temp_dir);
    }

    #[test]
    fn cache_path_ends_with_expected_filename() {
        let p = cache_path();
        assert!(p.ends_with("detectable_cache.json"));
    }

    #[test]
    fn parses_large_json_stream_exceeding_ten_megabytes() {
        let sample = r#"{"id":"123","name":"BigApp","executables":[{"name":"big.exe","os":"win32"}]},"#;
        let repeat_count = (11 * 1024 * 1024) / sample.len() + 1;
        let mut large_json = String::with_capacity(repeat_count * sample.len() + 2);
        large_json.push('[');
        for _ in 0..repeat_count {
            large_json.push_str(sample);
        }
        large_json.pop();
        large_json.push(']');

        assert!(large_json.len() > 10 * 1024 * 1024);

        let cursor = std::io::Cursor::new(large_json.into_bytes());
        let apps: Vec<DetectableApp> = serde_json::from_reader(cursor).expect("must parse payload > 10 MB without into_string limit");
        assert_eq!(apps.len(), repeat_count);
    }
}
