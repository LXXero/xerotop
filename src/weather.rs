//! Weather via wttr.in (no API key, no subprocess). A background thread fetches
//! on a slow timer — or immediately when the config changes — and streams a
//! parsed snapshot to GTK over a channel, mirroring the tray/taskbar pattern.

use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Default)]
pub struct Weather {
    pub icon: String,   // Nerd Font weather glyph
    pub temp: String,   // e.g. "+72°F"
    pub cond: String,   // e.g. "Partly cloudy"
    pub report: String, // multi-line detail for the tooltip
    pub ok: bool,
}

#[derive(Clone)]
pub struct WeatherReq {
    pub location: String, // empty = auto (wttr.in geolocates by IP)
    pub units: String,    // "auto" | "c" | "f"
    pub interval_min: f64,
}

/// Map a wttr.in condition string to a Nerd Font weather glyph.
fn glyph_for(cond: &str) -> &'static str {
    let c = cond.to_lowercase();
    if c.contains("thunder") || c.contains("storm") {
        "\u{e31d}"
    } else if c.contains("snow")
        || c.contains("sleet")
        || c.contains("blizzard")
        || c.contains("ice")
    {
        "\u{e31a}"
    } else if c.contains("rain") || c.contains("drizzle") || c.contains("shower") {
        "\u{e318}"
    } else if c.contains("fog") || c.contains("mist") || c.contains("haze") {
        "\u{e313}"
    } else if c.contains("overcast") || c.contains("cloud") {
        "\u{e312}"
    } else if c.contains("clear") || c.contains("sunny") || c.contains("sun") {
        "\u{e30d}"
    } else {
        "\u{e371}" // generic / unknown
    }
}

fn fetch(req: &WeatherReq) -> Option<Weather> {
    let loc: String = req
        .location
        .trim()
        .chars()
        .map(|c| if c == ' ' { '+' } else { c })
        .collect();
    let unit = match req.units.to_lowercase().as_str() {
        "f" => "&u",
        "c" => "&m",
        _ => "",
    };
    // condition | temp | location | feels-like | humidity | wind
    let url = format!("https://wttr.in/{loc}?format=%C|%t|%l|%f|%h|%w{unit}");
    let body = ureq::get(&url)
        .call()
        .ok()?
        .into_body()
        .read_to_string()
        .ok()?;
    let body = body.trim();
    // Guard against wttr.in error/HTML responses.
    if body.is_empty() || body.contains('<') || body.len() > 200 {
        return None;
    }
    let parts: Vec<&str> = body.split('|').map(str::trim).collect();
    let field = |i: usize| parts.get(i).copied().unwrap_or("");
    let (cond, temp, place, feels, humidity, wind) =
        (field(0), field(1), field(2), field(3), field(4), field(5));
    if temp.is_empty() {
        return None;
    }
    let mut report = String::new();
    if !place.is_empty() {
        report.push_str(place);
        report.push('\n');
    }
    report.push_str(&format!("{cond}  {temp}"));
    if !feels.is_empty() {
        report.push_str(&format!("  (feels {feels})"));
    }
    if !humidity.is_empty() || !wind.is_empty() {
        report.push_str(&format!("\n{humidity}  ·  {wind}"));
    }
    Some(Weather {
        icon: glyph_for(cond).to_string(),
        temp: temp.to_string(),
        cond: cond.to_string(),
        report,
        ok: true,
    })
}

/// Spawn the weather thread; returns (snapshot receiver, request sender). Send a
/// new `WeatherReq` to refetch immediately (e.g. when the location changes).
pub fn spawn(initial: WeatherReq) -> (async_channel::Receiver<Weather>, mpsc::Sender<WeatherReq>) {
    let (tx, rx) = async_channel::unbounded::<Weather>();
    let (req_tx, req_rx) = mpsc::channel::<WeatherReq>();
    std::thread::spawn(move || {
        let mut req = initial;
        loop {
            let _ = tx.send_blocking(fetch(&req).unwrap_or_default());
            let wait = Duration::from_secs_f64(req.interval_min.max(1.0) * 60.0);
            match req_rx.recv_timeout(wait) {
                Ok(new) => req = new,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    (rx, req_tx)
}
