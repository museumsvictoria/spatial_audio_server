use nannou::math::map_range;
use nannou::math::num_traits::{Float, NumCast};
use serde::{self, Deserialize, Serialize};
use serde_json;
use std::error::Error;
use std::io::Write;
use std::path::Path;
use std::time;
use std::{cmp, fmt, fs, io};
use time_calc::Ms;

pub const SEC_MS: f64 = 1_000.0;
pub const MIN_MS: f64 = SEC_MS * 60.0;
pub const HR_MS: f64 = MIN_MS * 60.0;
pub const DAY_MS: f64 = HR_MS * 24.0;

pub const MS_IN_HZ: f64 = 1_000.0;
pub const SEC_IN_HZ: f64 = 1.0;
pub const MIN_IN_HZ: f64 = SEC_IN_HZ / 60.0;
pub const HR_IN_HZ: f64 = MIN_IN_HZ / 60.0;
pub const DAY_IN_HZ: f64 = HR_IN_HZ / 24.0;

/// The type used to seed `XorShiftRng`s.
pub type Seed = [u8; 16];

/// Min and max values along a range.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Deserialize, Serialize)]
pub struct Range<T> {
    pub min: T,
    pub max: T,
}

impl<T> Range<T> {
    /// Clamp the given value to the range.
    pub fn clamp(&self, value: T) -> T
    where
        T: Clone + PartialOrd,
    {
        if value < self.min {
            self.min.clone()
        } else if value > self.max {
            self.max.clone()
        } else {
            value
        }
    }
}

pub enum HumanReadableTime {
    Ms,
    Secs,
    Mins,
    Hrs,
    Days,
}

impl HumanReadableTime {
    /// Convert the given "times per unit (self)" to hz.
    pub fn times_per_unit_to_hz(&self, times_per_unit: f64) -> f64 {
        match *self {
            HumanReadableTime::Ms => times_per_unit * MS_IN_HZ,
            HumanReadableTime::Secs => times_per_unit,
            HumanReadableTime::Mins => times_per_unit * MIN_IN_HZ,
            HumanReadableTime::Hrs => times_per_unit * HR_IN_HZ,
            HumanReadableTime::Days => times_per_unit * DAY_IN_HZ,
        }
    }

    /// Convert the given value to the next finer unit.
    ///
    /// If `self` is ms, ms is returned.
    pub fn to_finer_unit(&self, value: f64) -> (HumanReadableTime, f64) {
        match *self {
            HumanReadableTime::Ms => (HumanReadableTime::Ms, value),
            HumanReadableTime::Secs => (HumanReadableTime::Ms, value * SEC_MS),
            HumanReadableTime::Mins => (HumanReadableTime::Secs, value * 60.0),
            HumanReadableTime::Hrs => (HumanReadableTime::Mins, value * 60.0),
            HumanReadableTime::Days => (HumanReadableTime::Hrs, value * 24.0),
        }
    }

    /// Convert the given human readable value to Ms.
    pub fn to_ms(&self, value: f64) -> Ms {
        match *self {
            HumanReadableTime::Ms => Ms(value),
            HumanReadableTime::Secs => Ms(value * SEC_MS),
            HumanReadableTime::Mins => Ms(value * MIN_MS),
            HumanReadableTime::Hrs => Ms(value * HR_MS),
            HumanReadableTime::Days => Ms(value * DAY_MS),
        }
    }
}

/// Sums seed `b` onto seed `a` in a wrapping manner.
pub fn add_seeds(a: &Seed, b: &Seed) -> Seed {
    let mut result = [0; 16];
    for (r, (&a, &b)) in result.iter_mut().zip(a.iter().zip(b)) {
        *r = a.wrapping_add(b);
    }
    result
}

/// Count the number of elements that are equal to one another at the front.
pub fn count_equal<I, F>(iter: I, cmp: F) -> usize
where
    I: IntoIterator,
    F: Fn(&I::Item, &I::Item) -> cmp::Ordering,
{
    let mut count = 0;
    let mut iter = iter.into_iter().peekable();
    while let Some(item) = iter.next() {
        count += 1;
        if let Some(next) = iter.peek() {
            if let cmp::Ordering::Equal = cmp(&item, &next) {
                continue;
            }
        }
        break;
    }
    count
}

/// Convert the given interval in milliseconds to a rate in hz.
pub fn ms_interval_to_hz(ms: Ms) -> f64 {
    let secs_interval = ms.ms() / SEC_MS;
    let hz = 1.0 / secs_interval;
    hz
}

/// Convert the given rate in hz to an interval in milliseconds.
pub fn hz_to_ms_interval(hz: f64) -> Ms {
    let secs_interval = 1.0 / hz;
    let ms = Ms(secs_interval * SEC_MS);
    ms
}

/// Given a value in hz, produce a more readable "times per second".
///
/// E.g. a returned value of (Hrs, 3.5) can be thought of as "3.5 times per hour".
pub fn human_readable_hz(hz: f64) -> (HumanReadableTime, f64) {
    if hz < DAY_IN_HZ || hz < HR_IN_HZ {
        let times_per_day = hz / DAY_IN_HZ;
        (HumanReadableTime::Days, times_per_day)
    } else if hz < MIN_IN_HZ {
        let times_per_hr = hz / HR_IN_HZ;
        (HumanReadableTime::Hrs, times_per_hr)
    } else if hz < SEC_IN_HZ {
        let times_per_min = hz / MIN_IN_HZ;
        (HumanReadableTime::Mins, times_per_min)
    } else if hz < MS_IN_HZ {
        (HumanReadableTime::Secs, hz)
    } else {
        let times_per_ms = hz / MS_IN_HZ;
        (HumanReadableTime::Ms, times_per_ms)
    }
}

/// Given a number of milliseconds, produce the human readable time values.
pub fn human_readable_ms(ms: &Ms) -> (HumanReadableTime, f64) {
    let ms = ms.ms();
    if ms < SEC_MS {
        (HumanReadableTime::Ms, ms)
    } else if ms < MIN_MS {
        (HumanReadableTime::Secs, ms / SEC_MS)
    } else if ms < HR_MS {
        (HumanReadableTime::Mins, ms / MIN_MS)
    } else if ms < DAY_MS {
        (HumanReadableTime::Hrs, ms / HR_MS)
    } else {
        (HumanReadableTime::Days, ms / DAY_MS)
    }
}

/// Convert the given standard duration into its representation in seconds.
pub fn duration_to_secs(d: &time::Duration) -> f64 {
    d.as_secs() as f64 + d.subsec_nanos() as f64 * 1e-9
}

/// Given a path to some file, return whether or not it is a hidden file.
pub fn is_file_hidden<P>(path: P) -> bool
where
    P: AsRef<std::path::Path>,
{
    #[cfg(all(target_os = "windows", not(feature = "windows_metadataext")))]
    fn is_file_hidden_inner(_path: &std::path::Path) -> bool {
        false
    }

    #[cfg(all(target_os = "windows", feature = "windows_metadataext"))]
    /// Check if a file is hidden on windows, using the file attributes.
    /// To be enabled once windows::fs::MetadataExt is no longer an unstable API.
    fn is_file_hidden_inner(path: &std::path::Path) -> bool {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

        let metadata = std::fs::metadata(&path).ok();
        if let Some(metadata) = metadata {
            let win_attr: u32 = metadata.file_attributes();
            return (win_attr & FILE_ATTRIBUTE_HIDDEN) != 0;
        }
        false
    }

    #[cfg(not(target_os = "windows"))]
    /// Check if a file is hidden on any other OS than windows, using the dot file namings.
    fn is_file_hidden_inner(path: &std::path::Path) -> bool {
        let name = path.file_name();
        if let Some(name) = name {
            return name.to_string_lossy().starts_with(".");
        }
        false
    }

    is_file_hidden_inner(path.as_ref())
}

/// Errors that might occur when saving a file.
#[derive(Debug)]
pub enum FileError<E> {
    Io(io::Error),
    Format(E),
}

impl<E> From<io::Error> for FileError<E> {
    fn from(err: io::Error) -> Self {
        FileError::Io(err)
    }
}

impl From<serde_json::Error> for FileError<serde_json::Error> {
    fn from(err: serde_json::Error) -> Self {
        FileError::Format(err)
    }
}

impl<E> fmt::Display for FileError<E>
where
    E: Error,
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FileError::Io(ref err) => fmt::Display::fmt(err, f),
            FileError::Format(ref err) => fmt::Display::fmt(err, f),
        }
    }
}

/// Saves the file to a temporary file before removing the original to reduce the chance of losing
/// data in the case that something goes wrong during saving.
pub fn safe_file_save(path: &Path, content: &[u8]) -> io::Result<()> {
    let temp_path = path.with_extension("tmp");

    // If the temp file exists, remove it.
    if temp_path.exists() {
        fs::remove_file(&temp_path)?;
    }

    // Create the directory if it doesn't exist.
    if let Some(directory) = path.parent() {
        if !directory.exists() {
            fs::create_dir_all(&temp_path)?;
        }
    }

    // Write the temp file.
    let mut file = fs::File::create(&temp_path)?;
    file.write(content)?;

    // If there's already a file at `path`, remove it.
    if path.exists() {
        fs::remove_file(&path)?;
    }

    // Rename the temp file to the original path name.
    fs::rename(temp_path, path)?;

    Ok(())
}

/// A generic function for safely saving a serializable type to a JSON file.
pub fn save_to_json<T>(json_path: &Path, t: &T) -> Result<(), FileError<serde_json::Error>>
where
    T: serde::Serialize,
{
    let string = serde_json::to_string_pretty(t)?;
    safe_file_save(json_path, string.as_bytes())?;
    Ok(())
}

/// A generic funtion for loading a type from a JSON file.
pub fn load_from_json<'a, T>(json_path: &Path) -> Result<T, FileError<serde_json::Error>>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let file = fs::File::open(json_path)?;
    let t = serde_json::from_reader(file)?;
    Ok(t)
}

/// A generic function for loading a type from a json file.
///
/// If deserialization of file loading fails, a `Default` instance will be returned.
pub fn load_from_json_or_default<'a, T>(json_path: &Path) -> T
where
    T: for<'de> serde::Deserialize<'de> + Default,
{
    load_from_json(json_path).unwrap_or_else(|_| Default::default())
}

/// Unnormalise the given value.
pub fn unnormalise<T>(normalised_value: f64, min: T, max: T) -> T
where
    T: NumCast,
{
    map_range(normalised_value, 0.0, 1.0, min, max)
}

/// Unskew and unnormalise the given value.
pub fn unskew_and_unnormalise<T>(skewed_normalised_value: f64, min: T, max: T, skew: f32) -> T
where
    T: NumCast,
{
    let unskewed = skewed_normalised_value.powf(1.0 / skew as f64);
    unnormalise(unskewed, min, max)
}

/// Convert the given angle and magnitude into the x and y components of the representative vector.
pub fn rad_mag_to_x_y(rad: f64, mag: f64) -> (f64, f64) {
    let x = rad.cos() * mag;
    let y = rad.sin() * mag;
    (x, y)
}

/// Models the CPP fmod function.
#[inline]
pub fn fmod<F>(numer: F, denom: F) -> F
where
    F: Float,
{
    let rquot: F = (numer / denom).floor();
    numer - rquot * denom
}
