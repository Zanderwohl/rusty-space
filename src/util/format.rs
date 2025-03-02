

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
    let days = remaining_hours % 365;
    let remaining_years = (remaining_days - days) / 365;
    if remaining_years <= 0 {
        return format!("{}{}d {}h {}m {}s", sign, days, hours, mins, secs);
    }
    format!("{}{}y {}h {}m {}s", sign, days, hours, mins, secs)
}