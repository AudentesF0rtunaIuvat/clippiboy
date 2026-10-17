const FORBIDDEN: [char; 9] = ['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
const RESERVED: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "COM", "LPT"];
const MAX_LEN: usize = 60;
pub fn folder_name(game: &str) -> Option<String> {
    let cleaned: String = game
        .chars()
        .map(|c| if FORBIDDEN.contains(&c) || (c as u32) < 0x20 { ' ' } else { c })
        .collect();
    let mut name: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    name.truncate(name.char_indices().nth(MAX_LEN).map(|(at, _)| at).unwrap_or(name.len()));
    let name = name.trim_end_matches(['.', ' ']).to_string();
    if name.is_empty() { return None; }
    let stem = name.split('.').next().unwrap_or(&name).to_ascii_uppercase();
    let reserved = RESERVED.iter().any(|word| {
        stem == *word || (stem.len() == word.len() + 1 && stem.starts_with(word) && stem.ends_with(|c: char| c.is_ascii_digit()))
    });
    Some(if reserved { format!("_{name}") } else { name })
}
fn main() {
    for g in ["CON", "nul.txt", "COM1", "CONIN$", "CONOUT$", "COM\u{b9}", "LPT\u{b3}", "NUL .x", "COM1 .x", "Aux.", "NUL ."] {
        println!("{:?} -> {:?}", g, folder_name(g));
    }
    let a = "Pok\u{e9}mon"; let b = "Poke\u{301}mon";
    println!("{:?} {:?} equal={}", folder_name(a), folder_name(b), folder_name(a)==folder_name(b));
}
