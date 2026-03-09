/*
SPDX-License-Identifier: GPL-3.0-or-later

    Copyright (C) 2026  Elias Taufer

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

use chrono::{ NaiveTime, Local, Duration };
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::{self};
use log::{debug, warn};
use num_traits::cast::ToPrimitive;

use crate::serial::sender::{self, SenderMsg};
use crate::serial::reader::Reading;

#[derive(Debug)]
enum TimeOfDay { Sunrise, Day, Sunset, Night }

/// Messages the ControlActor can receive
#[derive(Debug)]
pub enum ControlMsg {
    SensorReading(Reading),
    GetSettings(oneshot::Sender<ControlSettings>),
    UpdateSettings(ControlSettingsPatch),
}

#[derive(Debug, Clone)]
pub struct ControlSettings {
    pub sunrise_start: NaiveTime,
    pub day_start: NaiveTime,
    pub sunset_start: NaiveTime,
    pub night_start: NaiveTime,
    pub day_temp: f64,
    pub night_temp: f64,
    pub day_light_level: f64,
    pub nigh_light_level: f64,
}

#[derive(Debug, Default, Clone)]
pub struct ControlSettingsPatch {
    pub sunrise_start: Option<NaiveTime>,
    pub day_start: Option<NaiveTime>,
    pub sunset_start: Option<NaiveTime>,
    pub night_start: Option<NaiveTime>,
    pub day_temp: Option<f64>,
    pub night_temp: Option<f64>,
    pub day_light_level: Option<f64>,
    pub nigh_light_level: Option<f64>,
}

#[derive(Clone)]
pub struct ControlHandle {
    tx: mpsc::UnboundedSender<ControlMsg>,
}

impl ControlHandle {
    pub fn send(&self, msg: ControlMsg) -> Result<(), mpsc::error::SendError<ControlMsg>> {
        self.tx.send(msg)
    }

    pub async fn get_settings(&self) -> Option<ControlSettings> {
        let (tx, rx) = oneshot::channel();
        let _ = self.tx.send(ControlMsg::GetSettings(tx));
        rx.await.ok()
    }

    pub fn update_settings(
        &self,
        patch: ControlSettingsPatch,
    ) -> Result<(), mpsc::error::SendError<ControlMsg>> {
        self.tx.send(ControlMsg::UpdateSettings(patch))
    }
}

pub struct ControlActor {
    rx: mpsc::UnboundedReceiver<ControlMsg>,

    sender: sender::SenderHandle,

    light_level: f64,
    target_temp: f64,

    time_of_day: TimeOfDay,
    sunrise_start: NaiveTime,
    day_start: NaiveTime,
    sunset_start: NaiveTime,
    night_start: NaiveTime,

    day_temp: f64,
    night_temp: f64,

    day_light_level: f64,
    night_light_level: f64,
}

impl ControlActor {
    pub fn spawn(sender: sender::SenderHandle) -> ControlHandle {
        let (tx, rx) = mpsc::unbounded_channel();

        let sunrise_start = NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        let day_start = sunrise_start + Duration::minutes(60);
        let sunset_start = NaiveTime::from_hms_opt(22, 55, 0).unwrap();
        let night_start = sunset_start + Duration::minutes(60);

        let now = Local::now().time(); 

        let time_of_day: TimeOfDay = match now {
            t if t >= sunrise_start && t < day_start => TimeOfDay::Sunrise,
            t if t >= day_start && t < sunset_start => TimeOfDay::Day,
            t if t >= sunset_start && t < night_start => TimeOfDay::Sunset,
            _ => TimeOfDay::Night,
        };     

        let day_temp = 23.0_f64;
        let night_temp = 22.0_f64;
        let target_temp = day_temp;

        let light_level = 0.0_f64;
        let day_light_level = 1.0_f64;
        let night_light_level = 0.4_f64;

        let actor = Self {
            rx,
            sender,
            light_level,
            target_temp,
            time_of_day,
            sunrise_start,
            day_start,
            sunset_start,
            night_start,
            day_temp,
            night_temp,
            day_light_level,
            night_light_level,
        };

        tokio::spawn(actor.run());

        ControlHandle { tx }
    }

    async fn run(mut self) {
        
        let mut tick = time::interval(std::time::Duration::from_secs(2));

        loop {
            tokio::select! {
                msg = self.rx.recv() => {
                    match msg {
                        Some(msg) => self.handle_msg(msg).await,
                        None => break, 
                    }
                }
                _ = tick.tick() => {
                    self.on_tick().await;
                }
            }
        }

        // todo: cleanup
    }

    async fn handle_msg(&mut self, msg: ControlMsg) {
        match msg {
            ControlMsg::SensorReading(sensor_reading) => self.on_sensor_reading(sensor_reading).await,
            ControlMsg::GetSettings(reply_to) => {
                let _ = reply_to.send(self.current_settings());
            }
            ControlMsg::UpdateSettings(patch) => {
                self.apply_settings_patch(patch);
            }
        }
    }

    async fn on_sensor_reading(&mut self, _sensor_reading: Reading) {
        // todo: implement 
    }

    async fn on_tick(&mut self) {

        let now = Local::now().time();

        debug!("Tick time of day: {:?}", self.time_of_day);

        match self.time_of_day {
            TimeOfDay::Sunrise => {
                if now > self.day_start {
                    self.time_of_day = TimeOfDay::Day;
                    return;
                }

                let sunrise_length = (self.day_start - self.sunrise_start).abs().num_milliseconds().to_f64().unwrap();
                let time_since_sunrise = (now - self.sunrise_start).abs().num_milliseconds().to_f64().unwrap();
                self.light_level = 
                    (time_since_sunrise / sunrise_length) * (self.day_light_level - self.night_light_level) 
                    + self.night_light_level;

                self.target_temp = self.day_temp;
            },
            TimeOfDay::Day => {
                if now > self.sunset_start {
                    self.time_of_day = TimeOfDay::Sunset;
                    return;
                }

                self.light_level = self.day_light_level;
                self.target_temp = self.day_temp;
            },
            TimeOfDay::Sunset => {
                if now < self.sunset_start || now >= self.night_start {
                    self.time_of_day = TimeOfDay::Night;
                    return;
                }

                let sunset_length = (self.night_start - self.sunset_start).abs().num_milliseconds().to_f64().unwrap();
                let time_since_sunset = (now - self.sunset_start).abs().num_milliseconds().to_f64().unwrap();
                self.light_level = 
                    (self.day_light_level - self.night_light_level) * (1.0f64 - time_since_sunset / sunset_length) 
                    + self.night_light_level;
                
                self.target_temp = self.night_temp;
            },
            TimeOfDay::Night => {
                if now >= self.sunrise_start && now < self.day_start {
                    self.time_of_day = TimeOfDay::Sunrise;
                    return;
                }

                self.light_level = self.night_light_level;
                self.target_temp = self.night_temp;
            },
        }

        self.write_heating_target_temp().await;
        self.write_light_level().await;
    }

    fn current_settings(&self) -> ControlSettings {
        ControlSettings {
            sunrise_start: self.sunrise_start,
            day_start: self.day_start,
            sunset_start: self.sunset_start,
            night_start: self.night_start,
            day_temp: self.day_temp,
            night_temp: self.night_temp,
            day_light_level: self.day_light_level,
            nigh_light_level: self.night_light_level,
        }
    }

    fn apply_settings_patch(&mut self, patch: ControlSettingsPatch) {
        if let Some(v) = patch.day_start {
            self.day_start = v;
        }
        if let Some(v) = patch.sunrise_start {
            self.sunrise_start = v;
        }
        if let Some(v) = patch.sunset_start {
            self.sunset_start = v;
        }
        if let Some(v) = patch.night_start {
            self.night_start = v;
        }
        if let Some(v) = patch.day_temp {
            self.day_temp = v;
        }
        if let Some(v) = patch.night_temp {
            self.night_temp = v;
        }
        if let Some(v) = patch.day_light_level {
            self.day_light_level = v.clamp(0.0, 1.0);
        }
        if let Some(v) = patch.nigh_light_level {
            self.night_light_level = v.clamp(0.0, 1.0);
        }

        self.time_of_day = time_of_day_from(Local::now().time(), self.sunrise_start, self.day_start, self.sunset_start, self.night_start);
    }

    async fn write_heating_target_temp(&mut self) {
        if let Err(e) = 
            self.sender.send_command(SenderMsg::TargetTemperature(self.target_temp as f32)) {
                warn!("Couldn't send message to sender.rs from control.rs: {e}");
        }
    }

    async fn write_light_level(&mut self) {
        if let Err(e) = 
            self.sender.send_command(SenderMsg::LEDBrightness(self.light_level as f32)) {
                warn!("Couldn't send message to sender.rs from control.rs: {e}");
        }
    }

}

fn time_of_day_from(
    now: NaiveTime,
    sunrise_start: NaiveTime,
    day_start: NaiveTime,
    sunset_start: NaiveTime,
    night_start: NaiveTime,
) -> TimeOfDay {
    match now {
        t if t >= sunrise_start && t < day_start => TimeOfDay::Sunrise,
        t if t >= day_start && t < sunset_start => TimeOfDay::Day,
        t if t >= sunset_start && t < night_start => TimeOfDay::Sunset,
        _ => TimeOfDay::Night,
    }
}
