//! Diagnostic: print what the phone would be told (top item + recent list). Reads only; opens nothing.
fn main() {
    let v = handoff_lib::capture::activity_report(true);
    println!("{}", serde_json::to_string_pretty(&v).unwrap());
}
