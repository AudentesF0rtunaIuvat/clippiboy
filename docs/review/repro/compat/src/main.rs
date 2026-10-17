#![allow(dead_code, unused_imports)]
mod v075; mod v076; mod v080; mod v082; mod v083;
use serde_json::Value;

fn head_config() -> v083::AppConfig {
    use v083::*;
    AppConfig {
        recording: RecordingConfig { target_kind: TargetKind::Window, target_id: Some("w1".into()), target_stable_id: Some("st".into()),
            width: 2560, height: 1440, fps: 144, bitrate_kbps: 30000, quality: 55, encoder: EncoderId::Qsv, keyframe_seconds: 3 },
        buffer: BufferConfig { auto_start: true, seconds: 300, clip_seconds: 45, memory_mb: 2048 },
        sources: vec![
            AudioSource { id: "g".into(), label: "Game".into(), kind: SourceKind::Game, enabled: true, gain_db: -3.5, muted: false, solo: false, separate_track: true },
            AudioSource { id: "p".into(), label: "Discord".into(), kind: SourceKind::Process { pid: 42, exe: Some("discord.exe".into()), mode: ProcessMode::Exclude }, enabled: true, gain_db: 2.0, muted: true, solo: true, separate_track: false },
            AudioSource { id: "m".into(), label: "Mic".into(), kind: SourceKind::InputDevice { device_id: "mic".into() }, enabled: false, gain_db: 0.0, muted: false, solo: false, separate_track: true },
        ],
        clip_dir: "D:\\Clips".into(),
        save_clip_hotkey: "F9".into(), toggle_buffer_hotkey: "".into(), screenshot_hotkey: "F10".into(), record_hotkey: "F11".into(), console_hotkey: "".into(),
        console_enabled: false, console_scale: 1.25, console_monitor: Some("\\\\.\\DISPLAY2".into()), console_monitor_stable_id: Some("x".into()),
        console_follow_active_screen: false, console_in_capture: true, console_glow: false, console_style: ConsoleStyle::Radial,
        auto_start_with_windows: true, only_buffer_in_game: false,
        overlay: OverlayConfig { enabled: false, on_clip_saved: false, on_buffer_toggle: false, on_error: false, on_screenshot: false, on_recording: false, rec_badge: false, corner: OverlayCorner::TopLeft, duration_ms: 1000, monitor: None, monitor_stable_id: None, follow_active_screen: true },
        tray_hint_shown: true, watermark: false, setup_done: true, usage_stats: false, usage_stats_noticed: true,
        control: ControlConfig { enabled: false, port: 50000, token: "tok".into() },
        accent_color: Some("#112233".into()), colors_unlocked: true, accent_mode: AccentMode::Rgb, accent_color_2: Some("#445566".into()), rgb_speed: RgbSpeed::Medium,
        friends: FriendsConfig { invisible: true, share_game: false, notify_requests: false, notify_online: true, notify_games: false, notify_while_playing: true, status: "afk".into(), busy: true, favorites: vec!["f1".into()], accept_clips: AcceptClips::Off, tag_friends: false, name_on_links: false },
        sounds: SoundConfig { enabled: false, volume: 0.1, ..Default::default() },
    }
}

fn diff(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x { match y.get(k) { Some(vb) => diff(&format!("{path}.{k}"), va, vb, out), None => out.push(format!("{path}.{k}: missing after")) } }
        }
        _ if a != b => out.push(format!("{path}: {a} -> {b}")),
        _ => {}
    }
}

macro_rules! roundtrip {
    ($m:ident, $json:expr) => {{
        let head_val: Value = serde_json::from_str($json).unwrap();
        match serde_json::from_str::<$m::AppConfig>($json) {
            Err(e) => println!("[{}] DOWNGRADE PARSE FAILED: {e}", stringify!($m)),
            Ok(old) => {
                let written = serde_json::to_string_pretty(&old).unwrap();
                match serde_json::from_str::<v083::AppConfig>(&written) {
                    Err(e) => println!("[{}] UPGRADE PARSE FAILED: {e}", stringify!($m)),
                    Ok(back) => {
                        let back_val = serde_json::to_value(&back).unwrap();
                        let mut out = Vec::new();
                        diff("", &head_val, &back_val, &mut out);
                        println!("[{}] downgrade ok, upgrade ok; values lost in round trip: {:?}", stringify!($m), out);
                    }
                }
            }
        }
    }};
}

fn main() {
    let cfg = head_config();
    let json = serde_json::to_string_pretty(&cfg).unwrap();
    roundtrip!(v082, &json);
    roundtrip!(v080, &json);
    roundtrip!(v076, &json);
    roundtrip!(v075, &json);

    // Non-finite floats in the config
    let mut bad = head_config();
    bad.console_scale = f64::INFINITY;
    let text = serde_json::to_string(&bad).unwrap();
    println!("inf console_scale serialized contains null: {}", text.contains("\"consoleScale\":null"));
    println!("parse back: {:?}", serde_json::from_str::<v083::AppConfig>(&text).err().map(|e| e.to_string()));

    // f64 -> f32 overflow from JSON
    let tm: v083::TrackMix = serde_json::from_str(r#"{"index":1,"gainDb":1e39,"muted":false}"#).unwrap();
    println!("gainDb 1e39 -> {}", tm.gain_db);
    let edit = v083::ClipEdit { start_ms: 0, end_ms: 1000, tracks: vec![tm] };
    let s = serde_json::to_string(&edit).unwrap();
    println!("edit stored as {s}; decode: {:?}", serde_json::from_str::<v083::ClipEdit>(&s).err().map(|e| e.to_string()));
}
