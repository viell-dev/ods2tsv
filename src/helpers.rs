/// Trim a String in place.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "trim_start cannot be larger than len"
)]
pub fn trim_in_place(s: &mut String) {
    s.truncate(s.trim_end().len()); // remove trailing
    let start = s.len() - s.trim_start().len();
    s.drain(..start); // remove leading
}
