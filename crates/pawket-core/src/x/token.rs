//! X embed `token`: a port of the JavaScript expression X's embed script uses.

/// The `token` parameter X's own embed script sends:
/// `((id / 1e15) * Math.PI).toString(36).replace(/(0+|\.)/g, "")`.
pub(super) fn syndication_token(post_id: &str) -> String {
    let value = post_id.parse::<f64>().unwrap_or(0.0) / 1e15 * std::f64::consts::PI;
    to_radix36(value).replace(['0', '.'], "")
}

const RADIX: f64 = 36.0;
const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// JavaScript's `Number.prototype.toString(36)` for finite non-negative
/// values below 2^53, ported from V8's `DoubleToRadixCString`, which emits the
/// shortest digit string that still identifies the double.
pub(super) fn to_radix36(value: f64) -> String {
    let mut integer = value.floor();
    let mut fraction = value - integer;
    let mut delta = (0.5 * (value.next_up() - value)).max(0.0_f64.next_up());
    let mut fraction_digits: Vec<u8> = Vec::new();
    if fraction >= delta {
        loop {
            fraction *= RADIX;
            delta *= RADIX;
            let digit = fraction as usize;
            fraction_digits.push(DIGITS[digit]);
            fraction -= digit as f64;
            let rounds_up = fraction > 0.5 || (fraction == 0.5 && digit % 2 == 1);
            if rounds_up && fraction + delta > 1.0 {
                round_up(&mut fraction_digits, &mut integer);
                break;
            }
            if fraction < delta {
                break;
            }
        }
    }
    let mut text = integer_to_radix36(integer as u64);
    if !fraction_digits.is_empty() {
        text.push('.');
        text.push_str(std::str::from_utf8(&fraction_digits).unwrap_or(""));
    }
    text
}

fn round_up(fraction_digits: &mut Vec<u8>, integer: &mut f64) {
    while let Some(last) = fraction_digits.pop() {
        let digit = DIGITS.iter().position(|d| *d == last).unwrap_or(0);
        if digit + 1 < DIGITS.len() {
            fraction_digits.push(DIGITS[digit + 1]);
            return;
        }
    }
    *integer += 1.0;
}

fn integer_to_radix36(mut value: u64) -> String {
    let mut digits = Vec::new();
    loop {
        digits.push(DIGITS[(value % 36) as usize]);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}
