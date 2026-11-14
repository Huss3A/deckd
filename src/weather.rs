//! Current weather for the wide slot's weather face. Uses Open-Meteo (free, no account or key) through the `curl.exe` that ships
//! with Windows, so no extra crate is needed. Only the city name typed in Settings is sent, and only while a weather face is
//! on the deck. Results are cached for 15 minutes; a fetch runs on a background thread and never blocks drawing.

use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct Weather {
    pub temp_c: f32,
    pub code: u32,
    pub is_day: bool,
    /// The place the city name resolved to.
    pub place: String,
}

struct State {
    city: String,
    at: Instant,
    data: Option<Weather>,
    fetching: bool,
    /// (lat, lon, name) once the city has been looked up.
    geo: Option<(f64, f64, String)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

const FRESH: Duration = Duration::from_secs(15 * 60);
const RETRY: Duration = Duration::from_secs(60);

/// The cached weather for `city` (None until the first fetch finishes, or when it failed). Starts a refresh when it is stale.
pub fn current(city: &str) -> Option<Weather> {
    let city = city.trim();
    if city.is_empty() {
        return None;
    }
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    let st = guard.get_or_insert_with(|| State { city: String::new(), at: Instant::now() - FRESH * 2, data: None, fetching: false, geo: None });
    if st.city != city {
        *st = State { city: city.to_string(), at: Instant::now() - FRESH * 2, data: None, fetching: false, geo: None };
    }
    let age = st.at.elapsed();
    let due = if st.data.is_some() { age >= FRESH } else { age >= RETRY };
    if due && !st.fetching {
        st.fetching = true;
        st.at = Instant::now();
        let (city, geo) = (st.city.clone(), st.geo.clone());
        std::thread::spawn(move || {
            let result = fetch(&city, geo);
            let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(st) = g.as_mut().filter(|s| s.city == city) {
                st.fetching = false;
                st.at = Instant::now();
                if let Ok((geo, w)) = result {
                    st.geo = Some(geo);
                    st.data = Some(w);
                }
            }
        });
    }
    st.data.clone()
}

fn curl(url: &str) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("curl.exe")
        .args(["-s", "--max-time", "10", url])
        .creation_flags(0x0800_0000) // no console window
        .output()
        .map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err("weather request failed".into());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

#[allow(clippy::type_complexity)]
fn fetch(city: &str, geo: Option<(f64, f64, String)>) -> Result<((f64, f64, String), Weather), String> {
    let (lat, lon, name) = match geo {
        Some(g) => g,
        None => {
            let body = curl(&format!("https://geocoding-api.open-meteo.com/v1/search?name={}&count=1", encode(city)))?;
            let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
            let r = v["results"].get(0).ok_or("city not found")?;
            (r["latitude"].as_f64().ok_or("no latitude")?, r["longitude"].as_f64().ok_or("no longitude")?, r["name"].as_str().unwrap_or(city).to_string())
        }
    };
    let body = curl(&format!("https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code,is_day"))?;
    let v: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let c = &v["current"];
    let w = Weather {
        temp_c: c["temperature_2m"].as_f64().ok_or("no temperature")? as f32,
        code: c["weather_code"].as_u64().unwrap_or(0) as u32,
        is_day: c["is_day"].as_u64().unwrap_or(1) == 1,
        place: name.clone(),
    };
    Ok(((lat, lon, name), w))
}

/// WMO weather code -> (symbol, words).
pub fn describe(code: u32, is_day: bool) -> (&'static str, &'static str) {
    match code {
        0 => (if is_day { "☀" } else { "☾" }, "Clear"),
        1 => (if is_day { "☀" } else { "☾" }, "Mostly clear"),
        2 => ("☁", "Partly cloudy"),
        3 => ("☁", "Overcast"),
        45 | 48 => ("☁", "Fog"),
        51..=57 => ("☂", "Drizzle"),
        61..=67 | 80..=82 => ("☂", "Rain"),
        71..=77 | 85 | 86 => ("❄", "Snow"),
        95..=99 => ("⚡", "Thunderstorm"),
        _ => ("☁", "Cloudy"),
    }
}
