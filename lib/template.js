/**
 * Replace {variable} placeholders in a template string.
 * Unknown variables are left as-is.
 */
export function interpolate(template, variables) {
  return template.replace(/\{(\w+)\}/g, (match, key) => {
    return key in variables ? String(variables[key]) : match;
  });
}

/**
 * Load a prompt template file and interpolate variables.
 */
export async function loadPrompt(promptPath, variables = {}) {
  const { readFile } = await import('node:fs/promises');
  const template = await readFile(promptPath, 'utf8');
  return interpolate(template, variables);
}
