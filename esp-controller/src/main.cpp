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

#include <Arduino.h>
#include <OneWire.h>
#include <DallasTemperature.h>
#include <QuickPID.h>

// PINS
const int ONE_WIRE_BUS     = 23;
const int HEATER_PIN       = 19;
const int LIGHT_PIN        = 18;

const int BAUD_RATE = 115200;
const int SERIAL_READ_BUFFER_LENGTH = 256;

// Update interval of esp controller
const int UPDATE_INTERVAL = 1000;

// light
const int LIGHT_PWM_CHANNEL = 2;
const int LIGHT_PWM_FREQ = 5000;
const int LIGHT_PWM_RES  = 10;
const float LIGHT_INIT_BRIGHTNESS = 1.0f;
static float ledBrightness = LIGHT_INIT_BRIGHTNESS; // Do not update manually! Change may not be applied
static float GAMMA_CORRECTION = 2.2f;

// Heating
const uint32_t CONTROL_INTERVAL_MS = 1000; // QuickPID sample time 
const uint32_t HEATER_WINDOW_MS    = 3000; // PWM window 
const float HEATER_BIAS = 20.0f;
const float WATER_TEMP_MAX = 28.0f;        // max for all aquarium residents
const float WATER_TEMP_SAFETY_MAX = 30.0f; // max for all equipment
const float WATER_TEMP_INIT_VAL = 24.0f;

class Heater {
  public:

    Heater();
    void init();
    void setSetpointC(float newSetpointC);
    void tick();
    void shutDown();

  private:

    float input = 0.0f;
    float output = 0.0f;
    float pidOutput = 0.0f;
    float setpointC = WATER_TEMP_INIT_VAL; 
    float pidSetpoint = WATER_TEMP_INIT_VAL * 100.0f;
    float Kp = 1.00f, Ki = 0.0006f, Kd = 0.0f;
    float POn = 1.0f;  
    float DOn = 0.0f;  
    bool pidLoop = false;
    QuickPID pid;

    void heaterTimeProportioning(float outputPercent);
};

Heater heater;

#include "esp_task_wdt.h"
constexpr int WDT_TIMEOUT_S = 3;

static void watchdogInit() {
  esp_task_wdt_init(WDT_TIMEOUT_S, true); // panic/reset on timeout
  esp_task_wdt_add(NULL);                 // watch the main loop task
}
static inline void watchdogFeed() { esp_task_wdt_reset(); }

// Temperature Sensor DS18B20
const float WATER_TEMP_CALIBRATION_VALUE = 0.375f;

class TempSensor {
  public:
    TempSensor();

    void init();
    void tick();

    float tempC = 0;

  private:
    OneWire oneWire;
    DallasTemperature waterTempSensor;

    float measureTempBlocking();
};

TempSensor tempSensor;

// helper functions
void serialWriteSensor(const char* sensor_id, const char* sensor_type, double value);
void serialReadCommands();
bool heaterSafetyOff = false; // of true heater can't be turned on
void heaterWrite(u_int32_t val);
int brightness_to_duty(float brightness);
void setBrighnessToPWM(float brightness);
void serialReport();

void setup() {
  Serial.begin(BAUD_RATE);

  pinMode(HEATER_PIN, OUTPUT);
  heaterWrite(LOW);

  watchdogInit();

  tempSensor.init();
  heater.init();

  ledcSetup(LIGHT_PWM_CHANNEL, LIGHT_PWM_FREQ, LIGHT_PWM_RES);
  ledcAttachPin(LIGHT_PIN, LIGHT_PWM_CHANNEL);
  setBrighnessToPWM(LIGHT_INIT_BRIGHTNESS);
}

void loop() {
  watchdogFeed();
  serialReadCommands();
  tempSensor.tick();
  heater.tick();
  serialReport();
}


// ========================
// Function implementations
// ========================

void serialReport() {
  static uint32_t lastReport = 0;
  if (millis() - lastReport >= UPDATE_INTERVAL) {
    lastReport += UPDATE_INTERVAL;
    serialWriteSensor("brightness", "light", ledBrightness);
  }
}

static inline char* ltrim(char* s) {
  while (*s && isspace((unsigned char)*s)) s++;
  return s;
}

static inline void rtrim_inplace(char* s) {
  size_t n = strlen(s);
  while (n && isspace((unsigned char)s[n - 1])) s[--n] = '\0';
}

static bool parseKeyValue(char* line, char*& key, char*& val) {
  line = ltrim(line);
  rtrim_inplace(line);
  if (*line == '\0') return false;

  // split key and value on first whitespace
  char* p = line;
  while (*p && !isspace((unsigned char)*p)) p++;
  if (*p == '\0') return false;           // no value present

  *p++ = '\0';
  p = ltrim(p);
  if (*p == '\0') return false;

  key = line;
  val = p;
  return true;
}

void serialWriteSensor(const char* sensor_id, const char* sensor_type, double value) {
  Serial.printf("sensor_id=%s sensor_type=%s value=%.4f\r\n", sensor_id, sensor_type, value);
}

void handleCommand(char *cmd) {
  char* key = nullptr;
  char* valStr = nullptr;

  if (!parseKeyValue(cmd, key, valStr)) return;

  char* valueEnd = nullptr;
  float value = strtof(valStr, &valueEnd);
  
  if (valueEnd == valStr) return;
  while (*valueEnd && isspace((unsigned char)*valueEnd)) valueEnd++;
  if (*valueEnd != '\0') return;

  if (strcmp(key, "target-temperature") == 0) {
    if (isfinite(value) || value > WATER_TEMP_MAX) {
      heater.setSetpointC(value);
    } else {
      serialWriteSensor("command-temp-too-high-or-invalid", "err", value);
    };
    serialWriteSensor("temp-command", "dbg", value);
    return;
  }

  if (strcmp(key, "led-brightness") == 0) {
    if (!isfinite(value)) return;
    if (value < 0.0f) value = 0.0f;
    if (value > 1.0f) value = 1.0f;
    setBrighnessToPWM(value);
    serialWriteSensor("brightness-command", "dbg", value);
    return;
  }
}

void serialReadCommands() {
  static char buffer[SERIAL_READ_BUFFER_LENGTH];
  static size_t bufferLength = 0;

  const uint32_t start = millis();
  const uint32_t budgetMs = 5;        // process serial for max 5ms per loop
  size_t processed = 0;

  while (Serial.available() > 0) {
    char c = (char)Serial.read();

    if (c == '\r') continue;  
    if (c == '\n') {          
      buffer[bufferLength] = '\0';
      handleCommand(buffer);   
      bufferLength = 0;
      continue;
    }

    if (bufferLength < sizeof(buffer) - 1) {
      buffer[bufferLength++] = c;
    } else {
      // overflow: drop line
      bufferLength = 0;
    }

    if ((millis() - start) >= budgetMs) break;
  }
}

int brightness_to_duty(float brightness) {
  const int maxDuty = (1 << LIGHT_PWM_RES) - 1;   
  brightness = constrain(brightness, 0.0f, 1.0f);

  float corrected = powf(brightness, GAMMA_CORRECTION);      
  int duty = (int)lroundf(corrected * maxDuty);

  return constrain(duty, 0, maxDuty);
}

void setBrighnessToPWM(float brightness) {
  brightness = constrain(brightness, 0.0f, 1.0f);
  ledBrightness = brightness;
  ledcWrite(LIGHT_PWM_CHANNEL, brightness_to_duty(brightness));
}

// ======
// Heater
// ======

void Heater::heaterTimeProportioning(float outputPercent) {
  static uint32_t windowStart = millis();
  uint32_t now = millis();

  outputPercent = constrain(outputPercent, 0.0f, 100.0f);
  uint32_t onTime = (uint32_t)lroundf((HEATER_WINDOW_MS * outputPercent) / 100.0f);
  uint32_t offTime = (uint32_t)HEATER_WINDOW_MS - onTime;

  if (outputPercent <= 0.0) {
    heaterWrite(LOW);
    return;
  }

  if (now - windowStart <= onTime) {
    heaterWrite(HIGH);
  } else if (now - windowStart <= onTime + offTime) {
    heaterWrite(LOW);
  } else {
    windowStart = now;   // reset
  }

}

void Heater::init() {
  pid.SetOutputLimits(- HEATER_BIAS,  60.0f - HEATER_BIAS );
  pid.SetSampleTimeUs((uint32_t)CONTROL_INTERVAL_MS * 1000);

  pid.SetMode(QuickPID::Control::automatic);
  pidLoop = true;
}

void Heater::setSetpointC(float newSetpointC) {
  setpointC = newSetpointC;
  pidSetpoint = newSetpointC * 100.0f;
}

Heater::Heater()
: pid(
  &input, &pidOutput, &pidSetpoint, Kp, Ki, Kd,
  QuickPID::pMode::pOnError,
  QuickPID::dMode::dOnMeas,
  QuickPID::iAwMode::iAwCondition,
  QuickPID::Action::direct
  )
{}

void Heater::tick() {

  input = tempSensor.tempC * 100.0f;

  if (pidLoop) {
    if (tempSensor.tempC < setpointC - 3.0f) {
      pid.SetMode(QuickPID::Control::manual);
      pidOutput = 40.0f;
    } else if (tempSensor.tempC > setpointC + 0.5f) {
      pid.SetMode(QuickPID::Control::manual);
      pidOutput = -HEATER_BIAS;
    } else {
      pid.SetMode(QuickPID::Control::automatic);
    }

    pid.Compute();

    output = HEATER_BIAS + pidOutput;

    if (tempSensor.tempC > setpointC + 1.0f || tempSensor.tempC > WATER_TEMP_MAX) { // extra layer of security to prevent overheating
      output = 0;
    }

    heaterTimeProportioning(output);
  } else {
    output = 0.0f;
    heaterTimeProportioning(0.0f);
    heaterWrite(LOW);
  }

  static uint32_t lastPrint = 0;
  if (millis() - lastPrint >= UPDATE_INTERVAL) {
    lastPrint += UPDATE_INTERVAL;
    serialWriteSensor("output", "internal_pid_val", output);
    serialWriteSensor("target", "internal_pid_val", setpointC);
  }

}

void Heater::shutDown() {
  pidLoop = false;
  output = 0.0f;
  heaterWrite(LOW);
}



// ==========
// TempSensor
// ==========

TempSensor::TempSensor() 
: oneWire(ONE_WIRE_BUS), 
  waterTempSensor(&oneWire)
{}

void TempSensor::init() {
  waterTempSensor.begin();
  waterTempSensor.setWaitForConversion(true);
  measureTempBlocking();
  waterTempSensor.setWaitForConversion(false);
}

float TempSensor::measureTempBlocking() {
  waterTempSensor.requestTemperaturesByIndex(0); 
  float waterTemp = waterTempSensor.getTempCByIndex(0); 
  waterTemp += WATER_TEMP_CALIBRATION_VALUE; 
  tempC = waterTemp;
  return tempC;
}

uint32_t convTimeMsForResolution(uint8_t res) {
  switch (res) {
    case 9:  return 94;
    case 10: return 188;
    case 11: return 375;
    default: return 750;
  }
}

void TempSensor::tick() {
  static uint32_t convStart = 0;
  static uint32_t conversionTime = convTimeMsForResolution(waterTempSensor.getResolution());
  enum class TempState { Idle, Converting };
  static TempState state = TempState::Idle;

  uint32_t now = millis();

  switch (state) {
    case TempState::Idle:
      if (now - convStart >= UPDATE_INTERVAL) {
        waterTempSensor.requestTemperaturesByIndex(0);
        convStart = now;
        state = TempState::Converting;
      }
      break;

    case TempState::Converting:
      if (now - convStart >= conversionTime) {
        float temp = waterTempSensor.getTempCByIndex(0);

        if (temp == DEVICE_DISCONNECTED_C || isnan(temp) || temp > WATER_TEMP_SAFETY_MAX) {
          heater.shutDown();
          heaterSafetyOff = true;
          Serial.println("DS18B20 error or too high -> heater off");
        }

        tempC = temp + WATER_TEMP_CALIBRATION_VALUE;

        serialWriteSensor("water_temp_01", "ds18b20", tempC);
        state = TempState::Idle; 
      }
      break;
  }
}

void heaterWrite(u_int32_t val) {
  if (heaterSafetyOff) {
    digitalWrite(HEATER_PIN, LOW);
    return;
  }

  digitalWrite(HEATER_PIN, val);
}