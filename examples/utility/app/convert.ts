// Unit conversion math. Plain functions, no signals: the screen owns the
// state and calls these on every read.
import { Math } from "plinth:core";

export const lengthUnits: string[] = ["meters", "feet", "miles", "kilometers"];
export const weightUnits: string[] = ["kilograms", "pounds", "ounces", "grams"];
export const temperatureUnits: string[] = ["Celsius", "Fahrenheit", "Kelvin"];
export const kinds: string[] = ["Length", "Weight", "Temperature"];

function lengthToMeters(value: number, unit: string): number {
  if (unit === "feet") {
    return value * 0.3048;
  }
  if (unit === "miles") {
    return value * 1609.344;
  }
  if (unit === "kilometers") {
    return value * 1000;
  }
  return value;
}

function metersTo(value: number, unit: string): number {
  if (unit === "feet") {
    return value / 0.3048;
  }
  if (unit === "miles") {
    return value / 1609.344;
  }
  if (unit === "kilometers") {
    return value / 1000;
  }
  return value;
}

function weightToGrams(value: number, unit: string): number {
  if (unit === "kilograms") {
    return value * 1000;
  }
  if (unit === "pounds") {
    return value * 453.59237;
  }
  if (unit === "ounces") {
    return value * 28.349523125;
  }
  return value;
}

function gramsTo(value: number, unit: string): number {
  if (unit === "kilograms") {
    return value / 1000;
  }
  if (unit === "pounds") {
    return value / 453.59237;
  }
  if (unit === "ounces") {
    return value / 28.349523125;
  }
  return value;
}

function temperatureToCelsius(value: number, unit: string): number {
  if (unit === "Fahrenheit") {
    return (value - 32) * (5 / 9);
  }
  if (unit === "Kelvin") {
    return value - 273.15;
  }
  return value;
}

function celsiusTo(value: number, unit: string): number {
  if (unit === "Fahrenheit") {
    return value * (9 / 5) + 32;
  }
  if (unit === "Kelvin") {
    return value + 273.15;
  }
  return value;
}

export function unitsFor(kind: string): string[] {
  if (kind === "Weight") {
    return weightUnits;
  }
  if (kind === "Temperature") {
    return temperatureUnits;
  }
  return lengthUnits;
}

/** Converts `value` from `fromUnit` to `toUnit`, within `kind`. Returns
 * `null` if either unit does not belong to `kind`. */
export function convert(kind: string, value: number, fromUnit: string, toUnit: string): number | null {
  if (kind === "Weight") {
    if (!weightUnits.includes(fromUnit) || !weightUnits.includes(toUnit)) {
      return null;
    }
    return gramsTo(weightToGrams(value, fromUnit), toUnit);
  }
  if (kind === "Temperature") {
    if (!temperatureUnits.includes(fromUnit) || !temperatureUnits.includes(toUnit)) {
      return null;
    }
    return celsiusTo(temperatureToCelsius(value, fromUnit), toUnit);
  }
  if (!lengthUnits.includes(fromUnit) || !lengthUnits.includes(toUnit)) {
    return null;
  }
  return metersTo(lengthToMeters(value, fromUnit), toUnit);
}

export function round(value: number): number {
  return Math.round(value * 1000) / 1000;
}
