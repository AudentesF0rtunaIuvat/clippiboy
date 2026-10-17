#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessMode { Include, Exclude }
#[derive(Debug, Clone, PartialEq)]
pub enum SourceKind { Game, Leftovers, OutputDevice { device_id: String }, InputDevice { device_id: String }, Process { pid: u32, exe: Option<String>, mode: ProcessMode } }
#[derive(Debug, Clone)]
pub struct AudioSource { pub id: String, pub label: String, pub kind: SourceKind, pub enabled: bool, pub gain_db: f32, pub muted: bool, pub solo: bool, pub separate_track: bool }
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKind { Output, Input }
#[derive(Debug, Clone)]
pub struct AudioDevice { pub id: String, pub name: String, pub kind: DeviceKind, pub is_default: bool }
