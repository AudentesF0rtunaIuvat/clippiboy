#![allow(dead_code, unused)]
mod model {
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessMode { Include, Exclude }
#[derive(Debug, Clone, PartialEq)]
pub enum SourceKind { Game, Leftovers, OutputDevice { device_id: String }, InputDevice { device_id: String }, Process { pid: u32, exe: Option<String>, mode: ProcessMode } }
#[derive(Debug, Clone)]
pub struct AudioSource { pub id: String, pub label: String, pub kind: SourceKind, pub enabled: bool, pub gain_db: f32, pub muted: bool, pub solo: bool, pub separate_track: bool }
}
mod devices { pub fn session_states(_: &str) -> Vec<(u32,bool)> { vec![] } }
mod log { macro_rules! info { ($($t:tt)*) => {} } pub(crate) use info; }
mod audio {
use crate::log;
use crate::devices;
use std::collections::HashMap;

use crate::model::{AudioSource, ProcessMode, SourceKind};

/// One WASAPI client the engine has to run, and the mixer source it feeds.
///
/// Several streams can carry the same `source_id`: the leftovers track is one
/// process loopback per application, because Windows can only ever name a single
/// process per client.
#[derive(Debug, Clone, PartialEq)]
pub struct Stream {
    pub source_id: String,
    pub kind: SourceKind,
}

/// A process's ancestors, closest first. `pid` itself is not included.
///
/// The depth cap is pure defence: a corrupt snapshot with a cycle would
/// otherwise hang the audio thread.
fn ancestors(pid: u32, tree: &HashMap<u32, (u32, String)>) -> Vec<u32> {
    let mut out = Vec::new();
    let mut current = pid;
    for _ in 0..32 {
        let Some((parent, _)) = tree.get(&current) else {
            break;
        };
        if *parent == 0 || out.contains(parent) {
            break;
        }
        out.push(*parent);
        current = *parent;
    }
    out
}

/// The top process of the application `pid` belongs to.
///
/// Upwards while the parent runs the same executable — Discord's audio sessions
/// live in utility children of one `Discord.exe`, and only their common parent
/// covers all of them in a single tap. Stopping at a differently named parent is
/// what keeps the walk from climbing out into `explorer.exe`.
pub fn app_root(pid: u32, tree: &HashMap<u32, (u32, String)>) -> u32 {
    let mut current = pid;
    for _ in 0..32 {
        let Some((parent, exe)) = tree.get(&current) else {
            break;
        };
        match tree.get(parent) {
            Some((_, parent_exe)) if parent_exe.eq_ignore_ascii_case(exe) => current = *parent,
            _ => break,
        }
    }
    current
}

/// Is anything being recorded application by application right now?
///
/// This is the whole switch. As long as nothing is, an output device is simply
/// that device's audio. The moment the game — or a single application — gets a
/// source of its own, recording the device whole would put that sound in the
/// clip twice, so every output device turns into its leftovers instead. Derived
/// rather than stored: there is nothing here a user could get wrong, and nothing
/// that can drift out of step with the sources.
pub fn records_single_applications(sources: &[AudioSource]) -> bool {
    sources.iter().filter(|s| s.enabled).any(|source| {
        matches!(
            source.kind,
            SourceKind::Game
                | SourceKind::Process {
                    mode: ProcessMode::Include,
                    ..
                }
        )
    })
}

/// What plays on each output device, as (pid, is it rendering right now).
///
/// Empty while nothing is recorded application by application — the enumeration
/// is a COM round trip per device and runs on the two-second tick.
pub fn sessions_by_device(sources: &[AudioSource]) -> HashMap<String, Vec<(u32, bool)>> {
    let mut out = HashMap::new();
    if !records_single_applications(sources) {
        return out;
    }
    for source in sources.iter().filter(|s| s.enabled) {
        if let SourceKind::OutputDevice { device_id } = &source.kind {
            out.entry(device_id.clone())
                .or_insert_with(|| devices::session_states(device_id));
        }
    }
    out
}

/// The live pid for a source that was pinned to a process.
///
/// A pid dies with the application, and the one in the configuration then
/// points at nothing — or, after Windows has recycled it, at a different
/// program altogether. So the stored pid counts only while a process with that
/// pid is still running *under the same exe name*; otherwise the exe name is
/// looked up afresh.
///
/// The same idea `game::still_running` applies to the game source, which has
/// had it all along — this is it for the applications picked by hand, which
/// until now stayed dead until somebody deleted and re-added them.
///
/// Where several processes share the name, the lowest pid wins: it is the
/// oldest, and with `INCLUDE_TARGET_PROCESS_TREE` a tap on it covers the
/// children anyway.
fn live_pid(pid: u32, exe: Option<&str>, tree: &HashMap<u32, (u32, String)>) -> Option<u32> {
    let matches_exe = |name: &str| {
        exe.is_some_and(|want| name.eq_ignore_ascii_case(want))
    };
    match tree.get(&pid) {
        // Still there and still itself.
        Some((_, name)) if exe.is_none() || matches_exe(name) => return Some(pid),
        // The pid is taken by something else, or gone. Fall through.
        _ => {}
    }
    let Some(exe) = exe else {
        // Nothing to search by — a source from before the exe was stored. It
        // keeps the old behaviour: the pid as it stands, and an error from the
        // stream if that pid is dead.
        return Some(pid);
    };
    let found = tree
        .iter()
        .filter(|(_, (_, name))| name.eq_ignore_ascii_case(exe))
        .map(|(pid, _)| *pid)
        .min();
    if let Some(now) = found {
        log::info!("'{exe}' is {now} now (was {pid}) — the source follows it");
    }
    found
}

/// Decides which leftovers source each application belongs to.
///
/// The point is that nobody should have to sort this out by hand. Windows says
/// whether a session is *rendering* or merely open, and that is exactly the
/// difference between "this application plays on that device" and "it once
/// opened a stream there and left it lying". So an application goes where it is
/// audibly playing, and everything else is only there to break ties.
#[derive(Default)]
pub struct Leftovers {
    /// The default output device — where anything not routed on purpose ends up,
    /// and therefore the right home for an application that is playing nowhere in
    /// particular.
    pub default_device: String,
    /// Who owned which application last time, so a moment of silence does not
    /// move it to another source and cost a stream restart.
    pub previous: HashMap<u32, String>,
}

impl Leftovers {
    fn assign(
        &self,
        sources: &[AudioSource],
        playing: &HashMap<String, Vec<(u32, bool)>>,
        tree: &HashMap<u32, (u32, String)>,
        taken: &[u32],
    ) -> Vec<(u32, String)> {
        // (source id, device id) of every source asking for leftovers, in the
        // order they sit in the mixer.
        let buckets: Vec<(&str, &str)> = sources
            .iter()
            .filter(|source| source.enabled)
            .filter_map(|source| match &source.kind {
                SourceKind::OutputDevice { device_id } => {
                    Some((source.id.as_str(), device_id.as_str()))
                }
                SourceKind::Leftovers => Some((source.id.as_str(), "")),
                _ => None,
            })
            .collect();

        // One entry per application rather than per session: Discord holds two,
        // and tapping both would split it for no gain. Ancestors first, so a
        // chosen tap already covers its own descendants when they come up.
        let mut roots: Vec<u32> = Vec::new();
        for sessions in playing.values() {
            for (pid, _) in sessions {
                let root = app_root(*pid, tree);
                if !roots.contains(&root) {
                    roots.push(root);
                }
            }
        }
        roots.sort_by_key(|root| ancestors(*root, tree).len());

        let mut out: Vec<(u32, String)> = Vec::new();
        let mut spoken_for: Vec<u32> = taken.to_vec();
        for root in roots {
            // A tap covers the target and its children, so anything below an
            // application another source records is already in the clip.
            let covered = spoken_for.contains(&root)
                || ancestors(root, tree)
                    .iter()
                    .any(|parent| spoken_for.contains(parent));
            if covered {
                continue;
            }

            // Does this application have a session on that bucket's device, and
            // is it rendering there?
            let state = |device: &str| {
                playing.get(device)?.iter().find_map(|(pid, active)| {
                    (app_root(*pid, tree) == root).then_some(*active)
                })
            };

            let owner = buckets
                .iter()
                // Where it is audibly playing. That is the whole rule.
                .find(|(_, device)| state(device) == Some(true))
                // It went quiet: stay put rather than move and cut the stream.
                .or_else(|| {
                    buckets.iter().find(|(id, device)| {
                        self.previous.get(&root).map(String::as_str) == Some(*id)
                            && state(device).is_some()
                    })
                })
                // Playing nowhere in particular belongs on the catch-all.
                .or_else(|| {
                    buckets
                        .iter()
                        .find(|(_, device)| *device == self.default_device && state(device).is_some())
                })
                .or_else(|| buckets.iter().find(|(_, device)| state(device).is_some()));

            if let Some((id, _)) = owner {
                spoken_for.push(root);
                out.push((root, (*id).to_string()));
            }
        }
        out
    }
}

/// Turns the stored, abstract sources into the streams the engine can open.
///
/// `playing` maps a device id to the processes holding a session on it, `tree`
/// is the process parentage both rules below need.
///
/// Two things cannot be expressed in the configuration and are only decided
/// here:
///
/// * **The game.** Its PID would be dead after the first restart of the game, so
///   it is filled in from the live detection. Without a detected game the source
///   **drops out** rather than staying on disabled — only that way does
///   `AudioEngine::apply` stop its stream instead of reporting a source that
///   cannot start. In the configuration it stays enabled, keeps its `TrackRing`
///   and therefore its track; the mixer pushes silence into it until a game
///   turns up.
///
/// * **The leftovers.** Windows has no "everything except these three" tap:
///   `AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS` names exactly one process. So they are
///   assembled from one tap per application on that device that no other source
///   records. Membership is decided by descent, not by an equal PID: a tap covers
///   the target *and its children*, so a parent and a child taken separately
///   would record the child twice.
///
///   Several devices may ask for the leftovers, and each application goes to
///   exactly one of them — see [`Leftovers`].
pub fn resolve(
    sources: &[AudioSource],
    game_pid: Option<u32>,
    playing: &HashMap<String, Vec<(u32, bool)>>,
    tree: &HashMap<u32, (u32, String)>,
    ledger: &Leftovers,
) -> Vec<Stream> {
    let enabled = || sources.iter().filter(|s| s.enabled);

    // Everything another source already records, plus ClippiBoy itself: playing a
    // clip back while the buffer runs must not end up in the next one. The
    // leftovers add to this as they go, which is what keeps several of them from
    // taking the same application.
    let mut taken: Vec<u32> = vec![std::process::id()];
    for source in enabled() {
        match &source.kind {
            SourceKind::Game => taken.extend(game_pid),
            // The live pid, not the stored one — otherwise a rebound source
            // would be tapped here and recorded by the leftovers as well.
            SourceKind::Process {
                pid,
                exe,
                mode: ProcessMode::Include,
            } => taken.extend(
                live_pid(*pid, exe.as_deref(), tree).map(|pid| app_root(pid, tree)),
            ),
            _ => {}
        }
    }

    // Which output device each application belongs to, decided once for all of
    // them so no two can claim the same one. Empty unless something is recorded
    // application by application — then a device is simply itself.
    let leftovers_mode = records_single_applications(sources);
    let home = ledger.assign(sources, playing, tree, &taken);

    let mut out = Vec::with_capacity(sources.len());
    for source in enabled() {
        let include = |pid: u32| Stream {
            source_id: source.id.clone(),
            kind: SourceKind::Process {
                pid,
                // Worked out fresh on every pass, so there is nothing to
                // remember it by.
                exe: None,
                mode: ProcessMode::Include,
            },
        };
        let leftovers = || {
            home.iter()
                .filter(|(_, owner)| owner == &source.id)
                .map(|(root, _)| include(*root))
                .collect::<Vec<_>>()
        };

        match &source.kind {
            SourceKind::Game => out.extend(game_pid.map(include)),
            SourceKind::OutputDevice { .. } if leftovers_mode => out.extend(leftovers()),
            // Only ever read from an old configuration; `config::migrate_sources`
            // turns it into a plain output device before it gets here.
            SourceKind::Leftovers => out.extend(leftovers()),
            // An application picked by hand: the stored pid is only a starting
            // point. Restarting Discord used to leave this source dead for good.
            SourceKind::Process { pid, exe, mode } => {
                if let Some(now) = live_pid(*pid, exe.as_deref(), tree) {
                    out.push(Stream {
                        source_id: source.id.clone(),
                        kind: SourceKind::Process {
                            pid: now,
                            exe: exe.clone(),
                            mode: *mode,
                        },
                    });
                }
            }
            other => out.push(Stream {
                source_id: source.id.clone(),
                kind: other.clone(),
            }),
        }
    }
    out
}
}
use std::collections::HashMap;
use model::*;
fn src(id:&str, kind: SourceKind) -> AudioSource { AudioSource{id:id.into(),label:id.into(),kind,enabled:true,gain_db:0.0,muted:false,solo:false,separate_track:true} }
fn main() {
    // explorer 100 -> steam.exe 33264 -> game.exe 5000 ; steamwebhelper 12604 child of steam
    let tree: HashMap<u32,(u32,String)> = [(100,(0,"explorer.exe")),(33264,(100,"steam.exe")),(12604,(33264,"steamwebhelper.exe")),(5000,(33264,"game.exe"))]
        .into_iter().map(|(p,(pp,e))| (p,(pp,e.to_string()))).collect();
    let sources = vec![src("game", SourceKind::Game), src("system", SourceKind::OutputDevice{device_id:"spk".into()})];
    let playing: HashMap<String, Vec<(u32,bool)>> = [("spk".to_string(), vec![(5000,true),(33264,false),(12604,false)])].into_iter().collect();
    let ledger = audio::Leftovers{ default_device: "spk".into(), previous: HashMap::new() };
    let streams = audio::resolve(&sources, Some(5000), &playing, &tree, &ledger);
    for s in &streams { println!("{:?}", s); }
    // Same-exe launcher stub: game.exe 6000 (stub, parent explorer) -> game.exe 6001 (foreground, holds session)
    let tree2: HashMap<u32,(u32,String)> = [(100,(0,"explorer.exe")),(6000,(100,"game.exe")),(6001,(6000,"game.exe"))]
        .into_iter().map(|(p,(pp,e))| (p,(pp,e.to_string()))).collect();
    let playing2: HashMap<String, Vec<(u32,bool)>> = [("spk".to_string(), vec![(6001,true)])].into_iter().collect();
    println!("--- stub case");
    for s in audio::resolve(&sources, Some(6001), &playing2, &tree2, &ledger) { println!("{:?}", s); }
}
