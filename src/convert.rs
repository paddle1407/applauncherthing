pub fn convert_input(input: &str) -> Option<String> {
    let input = input.trim();
    if input.is_empty() {
        return None;
    }

    let tokens: Vec<&str> = input.split_whitespace().collect();
    if tokens.len() < 3 {
        return None;
    }

    let amount: f64 = tokens[0].parse().ok()?;
    let (from_unit, to_unit) = if tokens.len() == 4 && (tokens[2] == "to" || tokens[2] == "in") {
        (tokens[1].to_lowercase(), tokens[3].to_lowercase())
    } else if tokens.len() == 3 {
        (tokens[1].to_lowercase(), tokens[2].to_lowercase())
    } else {
        return None;
    };

    let result = evaluate_conversion(amount, &from_unit, &to_unit)?;
    Some(format!(
        "{} {} = {} {}",
        format_num(amount),
        from_unit,
        format_num(result),
        to_unit
    ))
}

fn evaluate_conversion(val: f64, from: &str, to: &str) -> Option<f64> {
    match (from, to) {
        ("c" | "celsius", "f" | "fahrenheit") => return Some((val * 9.0 / 5.0) + 32.0),
        ("f" | "fahrenheit", "c" | "celsius") => return Some((val - 32.0) * 5.0 / 9.0),
        ("c" | "celsius", "k" | "kelvin") => return Some(val + 273.15),
        ("k" | "kelvin", "c" | "celsius") => return Some(val - 273.15),
        ("f" | "fahrenheit", "k" | "kelvin") => return Some((val - 32.0) * 5.0 / 9.0 + 273.15),
        ("k" | "kelvin", "f" | "fahrenheit") => return Some((val - 273.15) * 9.0 / 5.0 + 32.0),
        _ => {}
    }

    let length_factor = |u: &str| match u {
        "mm" | "millimeter" | "millimeters" => Some(0.001),
        "cm" | "centimeter" | "centimeters" => Some(0.01),
        "m" | "meter" | "meters" => Some(1.0),
        "km" | "kilometer" | "kilometers" => Some(1000.0),
        "in" | "inch" | "inches" => Some(0.0254),
        "ft" | "feet" | "foot" => Some(0.3048),
        "yd" | "yard" | "yards" => Some(0.9144),
        "mi" | "mile" | "miles" => Some(1609.344),
        _ => None,
    };

    if let (Some(f1), Some(f2)) = (length_factor(from), length_factor(to)) {
        return Some((val * f1) / f2);
    }

    let mass_factor = |u: &str| match u {
        "mg" | "milligram" | "milligrams" => Some(0.001),
        "g" | "gram" | "grams" => Some(1.0),
        "kg" | "kilogram" | "kilograms" => Some(1000.0),
        "oz" | "ounce" | "ounces" => Some(28.3495),
        "lb" | "lbs" | "pound" | "pounds" => Some(453.592),
        "t" | "ton" | "tons" => Some(1_000_000.0),
        _ => None,
    };

    if let (Some(f1), Some(f2)) = (mass_factor(from), mass_factor(to)) {
        return Some((val * f1) / f2);
    }

    let storage_factor = |u: &str| match u {
        "b" | "byte" | "bytes" => Some(1.0),
        "kb" | "kilobyte" | "kilobytes" => Some(1024.0),
        "mb" | "megabyte" | "megabytes" => Some(1024.0 * 1024.0),
        "gb" | "gigabyte" | "gigabytes" => Some(1024.0 * 1024.0 * 1024.0),
        "tb" | "terabyte" | "terabytes" => Some(1024.0 * 1024.0 * 1024.0 * 1024.0),
        _ => None,
    };

    if let (Some(f1), Some(f2)) = (storage_factor(from), storage_factor(to)) {
        return Some((val * f1) / f2);
    }

    let time_factor = |u: &str| match u {
        "s" | "sec" | "second" | "seconds" => Some(1.0),
        "m" | "min" | "minute" | "minutes" => Some(60.0),
        "h" | "hr" | "hour" | "hours" => Some(3600.0),
        "d" | "day" | "days" => Some(86400.0),
        "w" | "wk" | "week" | "weeks" => Some(604800.0),
        _ => None,
    };

    if let (Some(f1), Some(f2)) = (time_factor(from), time_factor(to)) {
        return Some((val * f1) / f2);
    }

    let speed_factor = |u: &str| match u {
        "m/s" => Some(1.0),
        "km/h" | "kph" => Some(1.0 / 3.6),
        "mph" => Some(0.44704),
        "knot" | "knots" => Some(0.514444),
        _ => None,
    };

    if let (Some(f1), Some(f2)) = (speed_factor(from), speed_factor(to)) {
        return Some((val * f1) / f2);
    }

    None
}

fn format_num(n: f64) -> String {
    if (n.fract() == 0.0 && n.abs() < 1e12) || n == 0.0 {
        format!("{:.0}", n)
    } else if n.abs() >= 100.0 {
        format!("{:.2}", n)
    } else if n.abs() >= 1.0 {
        format!("{:.3}", n)
    } else {
        format!("{:.4}", n)
    }
}
