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
}

#[derive(Clone)]
pub struct ControlHandle {
    tx: mpsc::UnboundedSender<ControlMsg>,
}

impl ControlHandle {
    pub fn send(&self, msg: ControlMsg) -> Result<(), mpsc::error::SendError<ControlMsg>> {
        self.tx.send(msg)
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
}

impl ControlActor {
    pub fn spawn(sender: sender::SenderHandle) -> ControlHandle {
        let (tx, rx) = mpsc::unbounded_channel();

        let sunrise_start = NaiveTime::from_hms_opt(7, 0, 0).unwrap();
        let day_start = sunrise_start + Duration::minutes(60);
        let sunset_start = NaiveTime::from_hms_opt(22, 30, 0).unwrap();
        let night_start = sunset_start + Duration::minutes(60);

        let now = Local::now().time(); 

        let time_of_day: TimeOfDay = match now {
            t if t >= sunrise_start && t < day_start => TimeOfDay::Sunrise,
            t if t >= day_start && t < sunset_start => TimeOfDay::Day,
            t if t >= sunset_start && t < night_start => TimeOfDay::Sunset,
            _ => TimeOfDay::Night,
        };     

        let day_temp = 0.0_f64;
        let night_temp = 0.0_f64;
        let target_temp = day_temp;

        let light_level = 0.0_f64;
        let day_light_level = 0.7_f64;

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
        };

        tokio::spawn(actor.run());

        ControlHandle { tx }
    }

    async fn run(mut self) {
        
        let mut tick = time::interval(std::time::Duration::from_secs(5));

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
                self.light_level = (time_since_sunrise / sunrise_length) * self.day_light_level;

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
                    self.day_light_level * (1.0f64 - time_since_sunset / sunset_length);
                
                self.target_temp = self.night_temp;
            },
            TimeOfDay::Night => {
                if now >= self.sunrise_start && now < self.day_start {
                    self.time_of_day = TimeOfDay::Sunrise;
                    return;
                }

                self.light_level = 0_f64;
                self.target_temp = self.night_temp;
            },
        }

        self.write_heating_target_temp().await;
        self.write_light_level().await;
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