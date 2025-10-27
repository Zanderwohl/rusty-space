use lazy_static::lazy_static;
use num_traits::Pow;
use regex::Regex;

pub fn seconds_to_naive_date(total_seconds: i64) -> String {
    let negative = total_seconds < 0;
    let sign = if negative { "-" } else { "" };
    let total_seconds = total_seconds.abs();
    
    let secs = total_seconds % 60;
    let remaining_mins = (total_seconds - secs) / 60;
    if remaining_mins <= 0 {
        return format!("{}{}s", sign, secs);
    }
    let mins = remaining_mins % 60;
    let remaining_hours = (remaining_mins - mins) / 60;
    if remaining_hours <= 0 {
        return format!("{}{}m {}s", sign, mins, secs);
    }
    let hours = remaining_hours % 24;
    let remaining_days = (remaining_hours - hours) / 24;
    if remaining_days <= 0 {
        return format!("{}{}h {}m {}s", sign, hours, mins, secs);
    }
    let days = remaining_days % 365;
    let remaining_years = (remaining_days as i128 - days as i128) / 365i128;
    if remaining_years <= 0 {
        return format!("{}{}d {}h {}m {}s", sign, days, hours, mins, secs);
    }
    format!("{}{}y {}d {}h {}m {}s", sign, remaining_years, days, hours, mins, secs)
}

pub fn sci_not(n: f64) -> String {
    if n.is_nan() {
        return "[NaN]".to_string();
    }

    let s = format!("{n:e}");
    let a = s.split("e").collect::<Vec<&str>>();
    if a.len() != 2 {
        panic!("sci_not failed with the values (n = {}) resulting in (a = {:?}).", n, a);
    }
    let mantissa = a[0].parse::<f64>().unwrap();
    let exponent = a[1].parse::<i64>().unwrap();
    format!("{:.3} x 10 ^ {}", mantissa, exponent)
}

lazy_static! {
    static ref SCI_RE: Regex = Regex::new(r"\d?\.\d+\s?x\s?10\s?\^\s?\d+").unwrap();
}

pub fn sci_not_parser(s: &str) -> Option<f64> {
    if !SCI_RE.is_match(s) {
        return None;
    }
    let s: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    let a = s.split("x").collect::<Vec<&str>>();
    let mantissa = a[0].parse::<f64>().ok()?;
    let b = a[1].split("^").collect::<Vec<&str>>();
    let exponent = b[1].parse::<i64>().ok()?;

    let result = mantissa * (10.0f64.pow(exponent as f64));

    Some(result)
}