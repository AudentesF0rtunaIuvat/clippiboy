use std::sync::Mutex;
static M: Mutex<Option<String>> = Mutex::new(None);
fn show(_a: Option<String>) { println!("inside show: locked = {}", M.try_lock().is_err()); }
fn main() {
    show(M.lock().unwrap().clone());
    show(Some(match M.lock().unwrap().as_ref() { Some(g) => g.clone(), None => "none".into() }));
    let g = M.lock().unwrap().clone(); show(g);
}
