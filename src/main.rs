#![warn(rust_2018_idioms)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_lossless,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss,
    clippy::similar_names,
    clippy::module_name_repetitions
)]

use std::{
    io::{ErrorKind, Read, Seek},
    path::PathBuf,
    process::ExitCode,
    sync::{atomic::AtomicBool, Arc},
};

use arraydeque::ArrayDeque;
use fan_controller::FanController;
use nonempty::NonEmpty as NonEmptyVec;
use signal_hook::consts::{SIGINT, SIGTERM};

use config::load_fan_configs;
use error::{Error, Result};

mod config;
mod error;
mod fan_controller;

#[cfg(not(target_os = "linux"))]
compile_error!("This tool is only developed for Linux systems.");

#[cfg(debug_assertions)]
const PID_FILE: &str = "t2fand.pid";
#[cfg(not(debug_assertions))]
const PID_FILE: &str = "/run/t2fand.pid";

fn get_current_euid() -> libc::uid_t {
    // SAFETY: FFI call with no preconditions
    unsafe { libc::geteuid() }
}

enum TempSource {
    Legacy {
        cpu_temp_file: std::fs::File,
        gpu_temp_file: Option<std::fs::File>,
    },
    MacSmc {
        temp_files: NonEmptyVec<std::fs::File>,
    },
}

fn is_apple_hwmon_name(name: &str) -> bool {
    matches!(name, "applesmc" | "macsmc-hwmon") || name.contains("macsmc")
}

fn find_apple_hwmon_paths() -> Result<Vec<PathBuf>> {
    let hwmon_paths = glob::glob("/sys/class/hwmon/hwmon*")?
        .filter_map(Result::ok)
        .filter(|path| {
            let Ok(name) = std::fs::read_to_string(path.join("name")) else {
                return false;
            };

            is_apple_hwmon_name(name.trim())
        })
        .collect();

    Ok(hwmon_paths)
}

fn find_fan_paths() -> Result<NonEmptyVec<PathBuf>> {
    let fan = glob::glob("/sys/devices/pci*/*/*/*/APP0001:00/fan*")?
        .filter_map(Result::ok)
        .find(|p| p.exists());

    if let Some(fan) = fan {
        let first_fan_path = fan.parent().ok_or(Error::NoFan)?;
        let fan_glob = first_fan_path.display().to_string() + "/fan*_input";
        let fans = glob::glob(&fan_glob)?
            .filter_map(Result::ok)
            .filter_map(|mut path| {
                let file_name = path.file_name()?.to_str()?;
                let fan_name = file_name.strip_suffix("_input")?;
                let fan_name_owned = fan_name.to_owned();
                path.set_file_name(fan_name_owned);
                Some(path)
            });

        if let Some(fans) = NonEmptyVec::collect(fans) {
            return Ok(fans);
        }
    }

    for hwmon_path in find_apple_hwmon_paths()? {
        let fan_glob = hwmon_path.display().to_string() + "/fan*_input";
        let fans = glob::glob(&fan_glob)?
            .filter_map(Result::ok)
            .filter_map(|mut path| {
                let file_name = path.file_name()?.to_str()?;
                let fan_name = file_name.strip_suffix("_input")?.to_owned();
                path.set_file_name(fan_name);
                Some(path)
            });

        if let Some(fans) = NonEmptyVec::collect(fans) {
            return Ok(fans);
        }
    }

    Err(Error::NoFan)
}

fn check_pid_file() -> Result<()> {
    match std::fs::read_to_string(PID_FILE) {
        Ok(pid) => {
            let mut proc_path = std::path::PathBuf::new();
            proc_path.push("/proc");
            proc_path.push(pid);

            if proc_path.exists() {
                return Err(Error::AlreadyRunning);
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(Error::PidRead(err)),
    }

    let current_pid = std::process::id().to_string();
    std::fs::write(PID_FILE, current_pid).map_err(Error::PidWrite)
}

fn read_temp_file(temp_file: &mut std::fs::File, temp_buf: &mut String) -> Result<u8> {
    temp_file
        .read_to_string(temp_buf)
        .map_err(Error::TempRead)?;

    temp_file.rewind().map_err(Error::TempSeek)?;

    let temp = temp_buf.trim_end().parse::<u32>().map_err(Error::TempParse);
    temp_buf.clear();
    temp.map(|t| (t / 1000) as u8)
}

fn open_temp_files(temps: glob::Paths, temp_buf: &mut String, temp_files: &mut Vec<std::fs::File>) {
    for temp_path_res in temps {
        let Ok(temp_path) = temp_path_res else {
            eprintln!("Unable to read glob path");
            continue;
        };

        let Ok(mut temp_file) = std::fs::File::open(temp_path) else {
            eprintln!("Unable to open temperature sensor");
            continue;
        };

        if read_temp_file(&mut temp_file, temp_buf).is_ok() {
            temp_files.push(temp_file);
        }
    }
}

fn find_temp_file(temps: glob::Paths, temp_buf: &mut String) -> Option<std::fs::File> {
    let mut temp_files = Vec::new();
    open_temp_files(temps, temp_buf, &mut temp_files);
    temp_files.into_iter().next()
}

fn find_cpu_temp_file(temp_buf: &mut String) -> Result<std::fs::File> {
    let temps = glob::glob("/sys/devices/platform/coretemp.0/hwmon/hwmon*/temp1_input")?;
    find_temp_file(temps, temp_buf).ok_or(Error::NoCpu)
}

fn find_gpu_temp_file(temp_buf: &mut String) -> Result<Option<std::fs::File>> {
    let temps = glob::glob("/sys/class/drm/card0/device/hwmon/hwmon*/temp1_input")?;
    Ok(find_temp_file(temps, temp_buf))
}

fn find_macsmc_temp_files(temp_buf: &mut String) -> Result<NonEmptyVec<std::fs::File>> {
    let mut temp_files = Vec::new();

    for hwmon_path in find_apple_hwmon_paths()? {
        let Ok(name) = std::fs::read_to_string(hwmon_path.join("name")) else {
            continue;
        };
        if !name.trim().contains("macsmc") {
            continue;
        }

        let temp_glob = hwmon_path.display().to_string() + "/temp*_input";
        open_temp_files(glob::glob(&temp_glob)?, temp_buf, &mut temp_files);
    }

    NonEmptyVec::from_vec(temp_files).ok_or(Error::NoCpu)
}

fn find_temp_source(temp_buf: &mut String) -> Result<TempSource> {
    match find_macsmc_temp_files(temp_buf) {
        Ok(temp_files) => Ok(TempSource::MacSmc { temp_files }),
        Err(Error::NoCpu) => Ok(TempSource::Legacy {
            cpu_temp_file: find_cpu_temp_file(temp_buf)?,
            gpu_temp_file: find_gpu_temp_file(temp_buf)?,
        }),
        Err(err) => Err(err),
    }
}

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("Error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn start_temp_loop(
    mut temp_buffer: String,
    mut temp_source: TempSource,
    fans: &NonEmptyVec<FanController>,
) -> Result<()> {
    let cancellation_token = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, cancellation_token.clone()).map_err(Error::Signal)?;
    signal_hook::flag::register(SIGTERM, cancellation_token.clone()).map_err(Error::Signal)?;

    let mut last_temp = 0;
    let mut temps = ArrayDeque::<u8, 50, arraydeque::Wrapping>::new();
    let mut was_long_sleep = false;
    while !cancellation_token.load(std::sync::atomic::Ordering::Relaxed) {
        let temp = match &mut temp_source {
            TempSource::Legacy {
                cpu_temp_file,
                gpu_temp_file,
            } => {
                let cpu_temp = read_temp_file(cpu_temp_file, &mut temp_buffer)?;
                if let Some(gpu_temp_file) = gpu_temp_file {
                    let gpu_temp = read_temp_file(gpu_temp_file, &mut temp_buffer)?;
                    if gpu_temp > cpu_temp {
                        gpu_temp
                    } else {
                        cpu_temp
                    }
                } else {
                    cpu_temp
                }
            }
            TempSource::MacSmc { temp_files } => {
                temp_files
                    .iter_mut()
                    .try_fold(0_u8, |max_temp, temp_file| {
                        read_temp_file(temp_file, &mut temp_buffer).map(|temp| {
                            if temp > max_temp {
                                temp
                            } else {
                                max_temp
                            }
                        })
                    })?
            }
        };

        temps.push_back(temp);
        if was_long_sleep {
            // Avoid messing up the mean due to the longer sleep.
            for _ in 0..9 {
                temps.push_back(temp);
            }
        }

        let sum_temp: u16 = temps.iter().map(|t| *t as u16).sum();
        let mean_temp = sum_temp / (temps.len() as u16);
        if mean_temp == last_temp {
            std::thread::sleep(std::time::Duration::from_secs(1));
            was_long_sleep = true;
        } else {
            last_temp = mean_temp;
            for fan in fans {
                fan.set_speed(fan.calc_speed(mean_temp as u8))?;
            }

            std::thread::sleep(std::time::Duration::from_millis(100));
            was_long_sleep = false;
        }
    }

    Ok(())
}

fn real_main() -> Result<()> {
    if get_current_euid() != 0 {
        return Err(Error::NotRoot);
    }

    check_pid_file()?;

    let mut temp_buffer = String::new();

    let fan_paths = find_fan_paths()?;
    let fans = load_fan_configs(fan_paths)?;
    let temp_source = find_temp_source(&mut temp_buffer)?;

    println!();
    for fan in &fans {
        fan.set_manual(true)?;
    }

    let res = start_temp_loop(temp_buffer, temp_source, &fans);
    println!("T2 Fan Daemon is shutting down...");
    for fan in fans {
        fan.set_manual(false)?;
    }

    let pid_res = std::fs::remove_file(PID_FILE).map_err(Error::PidDelete);
    match (res, pid_res) {
        (Err(err), _) | (_, Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}
