fn parse(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 { return None; }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}
const TITLE_SUFFIXES: &[&str] = &[" - steam"," - epic games"," - directx 12"," - directx 11"," - direct3d 12"," - direct3d 11"," - dx12"," - dx11"," - vulkan"," - opengl"," (64-bit)"," (32-bit)"," (x64)"," - 64 bit"," [steam]"];
fn strip(mut text: String) -> String {
    loop {
        let lower = text.to_lowercase();
        let Some(suffix) = TITLE_SUFFIXES.iter().find(|s| lower.ends_with(*s)) else { break; };
        text.truncate(text.len() - suffix.len());
        text = text.trim().to_string();
    }
    text
}
fn main() {
    // which non-ascii chars lowercase into something containing ascii
    for c in (0x80u32..0x110000).filter_map(char::from_u32) {
        let l: String = c.to_lowercase().collect();
        if l.chars().any(|x| x.is_ascii()) { println!("U+{:04X} {} -> {:?}", c as u32, c, l); }
    }
    println!("{:?}", strip("Foo - Vul\u{212A}an".into()));
    println!("{:?}", strip("Ab - Stea\u{2C6F}".into()));
    let r = std::panic::catch_unwind(|| parse("#\u{20ac}ab1"));
    println!("parse non-ascii panics: {}", r.is_err());
}
