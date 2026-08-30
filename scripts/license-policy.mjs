import parseSpdx from 'spdx-expression-parse';
import { Version } from '@cyclonedx/cyclonedx-library/Spec';
import { JsonValidator } from '@cyclonedx/cyclonedx-library/Validation';

export function validateSpdxExpression(expression, context) {
  if (typeof expression !== 'string' || !expression.trim()) throw new Error(`spdx_expression_missing:${context}`);
  const raw = expression.trim();
  const normalized = /^[A-Za-z0-9.+-]+(?:\s*\/\s*[A-Za-z0-9.+-]+)+$/.test(raw)
    ? raw.split(/\s*\/\s*/).join(' OR ')
    : raw;
  try {
    parseSpdx(normalized);
  } catch {
    throw new Error(`spdx_expression_invalid:${context}:${expression}`);
  }
  return normalized;
}

export async function validateCycloneDx16(jsonText) {
  const error = await new JsonValidator(Version.v1dot6).validate(jsonText);
  if (error !== null) throw new Error(`cyclonedx_1_6_invalid:${JSON.stringify(error)}`);
}
