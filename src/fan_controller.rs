use std::{
    io::{IsTerminal, Seek, Write},
    path::PathBuf,
};

use crate::{
    config::{FanConfig, SpeedCurve},
    error::{Error, Result},
};

#[derive(Debug)]
enum FanControl {
    ManualOutput {
        manual_file: std::fs::File,
        output_file: std::fs::File,
    },
    Target(std::fs::File),
}

#[derive(Debug)]
pub struct FanController {
    control: FanControl,
    config: FanConfig,

    min_speed: u32,
    max_speed: u32,
}

impl FanController {
    fn open_writable(open_options: &std::fs::OpenOptions, path: PathBuf) -> Result<std::fs::File> {
        open_options.open(&path).map_err(|source| Error::FanOpen {
            path: path.display().to_string(),
            source,
        })
    }

    fn write_file(file: &std::fs::File, value: &[u8]) -> Result<()> {
        let mut file = file;
        file.rewind().map_err(Error::FanWrite)?;
        file.write_all(value).map_err(Error::FanWrite)
    }

    pub fn new(path: PathBuf, config: FanConfig) -> Result<Self> {
        fn join_suffix(mut path: PathBuf, suffix: &str) -> PathBuf {
            let file_name = path.file_name().unwrap().to_str().unwrap();
            path.set_file_name(format!("{file_name}{suffix}"));
            path
        }

        let min_speed = std::fs::read_to_string(join_suffix(path.clone(), "_min"))
            .map_err(Error::MinSpeedRead)?
            .trim()
            .parse()
            .map_err(Error::MinSpeedParse)?;

        let max_speed = std::fs::read_to_string(join_suffix(path.clone(), "_max"))
            .map_err(Error::MaxSpeedRead)?
            .trim_end()
            .parse()
            .map_err(Error::MaxSpeedParse)?;

        let mut open_options = std::fs::OpenOptions::new();
        open_options.write(true);

        let target_path = join_suffix(path.clone(), "_target");
        let control = if target_path.exists() {
            FanControl::Target(Self::open_writable(&open_options, target_path)?)
        } else {
            let manual_file =
                Self::open_writable(&open_options, join_suffix(path.clone(), "_manual"))?;
            let output_file = Self::open_writable(&open_options, join_suffix(path, "_output"))?;

            FanControl::ManualOutput {
                manual_file,
                output_file,
            }
        };

        let this = Self {
            control,
            config,
            min_speed,
            max_speed,
        };

        println!("Found fan: {this:#?}");
        Ok(this)
    }

    pub fn set_manual(&self, enabled: bool) -> Result<()> {
        match &self.control {
            FanControl::ManualOutput { manual_file, .. } => {
                Self::write_file(manual_file, if enabled { b"1" } else { b"0" })
            }
            FanControl::Target(_) => Ok(()),
        }
    }

    pub fn set_speed(&self, mut speed: u32) -> Result<()> {
        if speed < self.min_speed {
            speed = self.min_speed;
        } else if speed > self.max_speed {
            speed = self.max_speed;
        }

        {
            let mut stdout = std::io::stdout().lock();
            if stdout.is_terminal() {
                print!("\x1b[1K\rSetting fan speed to {speed}");
                let _ = stdout.flush();
            }
        }

        let speed = speed.to_string();
        match &self.control {
            FanControl::ManualOutput { output_file, .. } => {
                Self::write_file(output_file, speed.as_bytes())
            }
            FanControl::Target(target_file) => Self::write_file(target_file, speed.as_bytes()),
        }
    }

    pub fn calc_speed(&self, temp: u8) -> u32 {
        if self.config.always_full_speed {
            return self.max_speed;
        }

        if temp <= self.config.low_temp {
            return self.min_speed;
        }
        if temp >= self.config.high_temp {
            return self.max_speed;
        }

        let temp = temp as u32;
        let low_temp = self.config.low_temp as u32;
        let high_temp = self.config.high_temp as u32;
        match self.config.speed_curve {
            SpeedCurve::Linear => {
                ((temp - low_temp) as f32 / (high_temp - low_temp) as f32
                    * (self.max_speed - self.min_speed) as f32) as u32
                    + self.min_speed
            }
            SpeedCurve::Exponential => {
                ((temp - low_temp).pow(3) as f32 / (high_temp - low_temp).pow(3) as f32
                    * (self.max_speed - self.min_speed) as f32) as u32
                    + self.min_speed
            }
            SpeedCurve::Logarithmic => {
                (((temp - low_temp) as f32).log((high_temp - low_temp) as f32)
                    * (self.max_speed - self.min_speed) as f32) as u32
                    + self.min_speed
            }
        }
    }
}
