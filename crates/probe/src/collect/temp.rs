//! Temperatures from `/sys/class/hwmon`.

use serde::Serialize;
use std::fs;
use std::path::PathBuf;

pub const MAX_SENSORS: usize = 8;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Sensor {
    pub name: String,
    pub celsius: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Temperature {
    /// The representative value used for charts and alerts.
    pub celsius: f64,
    pub sensors: Vec<Sensor>,
}

/// One `tempN_input` file and what it is called.
#[derive(Debug, Clone)]
pub struct SensorFile {
    pub chip: String,
    pub label: String,
    pub path: PathBuf,
}

fn plausible(c: f64) -> bool {
    c.is_finite() && c > -40.0 && c < 150.0
}

/// Rank for picking the representative sensor; lower is better.
fn rank(chip: &str, label: &str) -> Option<u8> {
    match (chip, label) {
        ("coretemp", l) if l.starts_with("Package id 0") => Some(0),
        ("k10temp" | "zenpower", "Tdie") => Some(1),
        ("k10temp" | "zenpower", "Tctl") => Some(2),
        ("cpu_thermal" | "cpu-thermal" | "soc_thermal", _) => Some(3),
        _ => None,
    }
}

/// Pick the representative value and list at most [`MAX_SENSORS`]
/// readings, representative first. `None` when nothing plausible was read.
pub fn summarise(readings: &[(String, String, f64)]) -> Option<Temperature> {
    let valid: Vec<&(String, String, f64)> =
        readings.iter().filter(|(_, _, c)| plausible(*c)).collect();
    if valid.is_empty() {
        return None;
    }
    let best = valid
        .iter()
        .filter_map(|r| rank(&r.0, &r.1).map(|k| (k, *r)))
        .min_by_key(|(k, _)| *k)
        .map(|(_, r)| r)
        .unwrap_or_else(|| {
            valid
                .iter()
                .max_by(|a, b| a.2.total_cmp(&b.2))
                .copied()
                .unwrap()
        });
    let name = |r: &(String, String, f64)| format!("{} {}", r.0, r.1).trim().to_string();
    let mut sensors = vec![Sensor {
        name: name(best),
        celsius: best.2,
    }];
    for r in &valid {
        if sensors.len() >= MAX_SENSORS {
            break;
        }
        if !std::ptr::eq(*r, best) {
            sensors.push(Sensor {
                name: name(r),
                celsius: r.2,
            });
        }
    }
    Some(Temperature {
        celsius: best.2,
        sensors,
    })
}

fn read_trimmed(path: &PathBuf) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Find every `tempN_input` under `/sys/class/hwmon`. Done at start and
/// once an hour, since sensors rarely come and go.
pub fn scan() -> Vec<SensorFile> {
    let Ok(dirs) = fs::read_dir("/sys/class/hwmon") else {
        return Vec::new();
    };
    let mut files = Vec::new();
    let mut dirs: Vec<PathBuf> = dirs.filter_map(|d| d.ok().map(|d| d.path())).collect();
    dirs.sort();
    for dir in dirs {
        let chip = read_trimmed(&dir.join("name")).unwrap_or_default();
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut inputs: Vec<String> = entries
            .filter_map(|e| e.ok()?.file_name().into_string().ok())
            .filter(|n| n.starts_with("temp") && n.ends_with("_input"))
            .collect();
        inputs.sort();
        for input in inputs {
            let index = &input["temp".len()..input.len() - "_input".len()];
            let label = read_trimmed(&dir.join(format!("temp{index}_label")))
                .unwrap_or_else(|| format!("temp{index}"));
            files.push(SensorFile {
                chip: chip.clone(),
                label,
                path: dir.join(&input),
            });
        }
    }
    files
}

pub fn read(files: &[SensorFile]) -> Option<Temperature> {
    let readings: Vec<(String, String, f64)> = files
        .iter()
        .filter_map(|f| {
            let milli: f64 = read_trimmed(&f.path)?.parse().ok()?;
            Some((f.chip.clone(), f.label.clone(), milli / 1000.0))
        })
        .collect();
    summarise(&readings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(chip: &str, label: &str, c: f64) -> (String, String, f64) {
        (chip.into(), label.into(), c)
    }

    #[test]
    fn prefers_intel_package() {
        let t = summarise(&[
            r("nvme", "Composite", 61.0),
            r("coretemp", "Core 0", 50.0),
            r("coretemp", "Package id 0", 54.0),
        ])
        .unwrap();
        assert_eq!(t.celsius, 54.0);
        assert_eq!(t.sensors[0].name, "coretemp Package id 0");
        assert_eq!(t.sensors.len(), 3);
    }

    #[test]
    fn prefers_amd_tdie_over_tctl() {
        let t = summarise(&[r("k10temp", "Tctl", 70.0), r("k10temp", "Tdie", 60.0)]).unwrap();
        assert_eq!(t.celsius, 60.0);
    }

    #[test]
    fn raspberry_pi() {
        let t = summarise(&[r("cpu_thermal", "temp1", 48.3)]).unwrap();
        assert_eq!(t.celsius, 48.3);
    }

    #[test]
    fn falls_back_to_highest_and_ignores_nonsense() {
        let t = summarise(&[
            r("acpitz", "temp1", 27.8),
            r("nvme", "Composite", 41.0),
            r("weird", "temp1", -273.0),
            r("weird", "temp2", 511.0),
        ])
        .unwrap();
        assert_eq!(t.celsius, 41.0);
        assert_eq!(t.sensors.len(), 2);
    }

    #[test]
    fn nothing_readable_means_no_temperature() {
        assert_eq!(summarise(&[]), None);
        assert_eq!(summarise(&[r("x", "y", f64::NAN)]), None);
    }

    #[test]
    fn caps_the_list() {
        let many: Vec<_> = (0..20)
            .map(|i| r("coretemp", &format!("Core {i}"), 40.0))
            .collect();
        assert_eq!(summarise(&many).unwrap().sensors.len(), MAX_SENSORS);
    }
}
