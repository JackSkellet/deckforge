//! Independently drawn volume meters; no vendor artwork or remote resources.
fn escaped(value: &str) -> String {
    value
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(20)
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn svg(value: Option<f64>, maximum: f64, vertical: bool, muted: bool, label: &str) -> String {
    let maximum = if maximum.is_finite() && maximum > 0.0 {
        maximum
    } else {
        100.0
    };
    let value = value
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, maximum));
    let ratio = value.unwrap_or(0.0) / maximum;
    let color = if muted { "#ed998e" } else { "#8ae0c7" };
    let number = value
        .map(|v| format!("{v:.0}%"))
        .unwrap_or_else(|| "--".into());
    let meter = if vertical {
        format!(
            r##"<rect x="114" y="46" width="10" height="76" rx="5" fill="#394052"/><rect x="114" y="{:.2}" width="10" height="{:.2}" rx="5" fill="{color}"/>"##,
            122.0 - 76.0 * ratio,
            76.0 * ratio
        )
    } else {
        format!(
            r##"<rect x="16" y="94" width="112" height="10" rx="5" fill="#394052"/><rect x="16" y="94" width="{:.2}" height="10" rx="5" fill="{color}"/>"##,
            112.0 * ratio
        )
    };
    let note = if muted {
        "Muted"
    } else if value.is_none() {
        "Unavailable"
    } else {
        ""
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="144" height="144" viewBox="0 0 144 144"><rect width="144" height="144" rx="16" fill="#171b24"/><text x="72" y="27" text-anchor="middle" font-family="sans-serif" font-size="14" fill="#dce3ef">{}</text><text x="{}" y="74" text-anchor="middle" font-family="sans-serif" font-size="28" fill="{color}">{number}</text>{meter}<text x="72" y="134" text-anchor="middle" font-family="sans-serif" font-size="12" fill="#dce3ef">{note}</text></svg>"##,
        escaped(label),
        if vertical { 60 } else { 72 }
    )
}

pub fn fader(value: Option<f64>, maximum: f64, vertical: bool, muted: bool, label: &str) -> String {
    use std::fmt::Write;
    let mut uri = String::from("data:image/svg+xml,");
    for byte in svg(value, maximum, vertical, muted, label).bytes() {
        write!(&mut uri, "%{byte:02X}").unwrap();
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn orientations_and_ranges_are_visible_in_geometry() {
        assert!(svg(Some(50.0), 100.0, false, false, "Volume").contains("width=\"56.00\""));
        let vertical = svg(Some(100.0), 200.0, true, false, "Volume");
        assert!(vertical.contains("y=\"84.00\"") && vertical.contains("height=\"38.00\""));
        assert!(svg(Some(900.0), 200.0, false, false, "Volume").contains(">200%</text>"));
        assert!(svg(Some(-1.0), 200.0, false, false, "Volume").contains("width=\"0.00\""));
    }
    #[test]
    fn unavailable_mute_and_escaped_labels_do_not_invent_volume_or_markup() {
        let text = svg(Some(f64::NAN), 100.0, false, false, "<user>&");
        assert!(
            text.contains("&lt;user&gt;&amp;")
                && text.contains("Unavailable")
                && text.contains(">--</text>")
        );
        let text = svg(Some(75.0), 100.0, false, true, "Volume");
        assert!(text.contains("Muted") && text.contains(">75%</text>"));
        let uri = fader(Some(1.0), 100.0, false, false, "Volume");
        assert!(uri.starts_with("data:image/svg+xml,%3C%73%76%67"));
    }
}
