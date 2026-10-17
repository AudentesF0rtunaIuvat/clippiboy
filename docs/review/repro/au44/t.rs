const EDGE_TOLERANCE_MS: u64 = 250;
const MIN_LENGTH_MS: u64 = 200;
fn plan(offset: u64, full: u64, d: u64, ts: u64, te: u64, has_original: bool) {
    let start = ts.min(d);
    let end = te.clamp(start + MIN_LENGTH_MS, d.max(start + MIN_LENGTH_MS));
    let cuts_front = start > EDGE_TOLERANCE_MS;
    let cuts_back = end + EDGE_TOLERANCE_MS < d;
    let absolute_start = offset + if cuts_front { start } else { 0 };
    let absolute_end = offset + if cuts_back { end } else { d };
    let length_ms = absolute_end.saturating_sub(absolute_start).max(MIN_LENGTH_MS);
    let video_start = match (cuts_front, has_original) { (false, _) => 0, (true, true) => absolute_start, (true, false) => start };
    let trimmed = absolute_start > 0 || absolute_end < full;
    println!("in ({ts},{te}) D={d} off={offset}: start={start} end={end} abs=({absolute_start},{absolute_end}) len={length_ms} vstart={video_start} trimmed={trimmed}");
}
fn main() {
    plan(0, 30000, 30000, 30000, 30000, true);
    plan(0, 30000, 30000, 29950, 30000, true);
    plan(0, 30000, 30000, 29500, 30000, true);
    plan(5000, 60000, 30000, 30000, 30000, true);
}
