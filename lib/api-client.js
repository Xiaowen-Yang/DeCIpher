/**
 * AI API client supporting OpenAI and Anthropic providers.
 * Reads provider + api_key + model from config passed in.
 */

const OPENAI_URL = "https://api.openai.com/v1/chat/completions";
const ANTHROPIC_URL = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION = "2023-06-01";

/**
 * Call the configured AI provider with a prompt.
 * @param {string} prompt - Full prompt text
 * @param {object} config - { provider, api_key, model }
 * @returns {Promise<string>} - The AI response text
 */
export async function callAI(prompt, config) {
  const { provider, api_key, model, base_url } = config;

  if (!api_key) {
    throw new Error(
      "No API key configured. Run: decipher config set api-key <key>",
    );
  }

  // Custom base URL = always use OpenAI-compatible format
  if (base_url) {
    return callOpenAI(prompt, api_key, model ?? "gpt-4o", base_url);
  }

  if (provider === "anthropic") {
    return callAnthropic(prompt, api_key, model ?? "claude-sonnet-4-20250514");
  }

  return callOpenAI(prompt, api_key, model ?? "gpt-4o", OPENAI_URL);
}

async function callOpenAI(prompt, apiKey, model, url) {
  const response = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${apiKey}`,
    },
    body: JSON.stringify({
      model,
      messages: [{ role: "user", content: prompt }],
      temperature: 0,
    }),
  });

  if (!response.ok) {
    const error = await response.text();
    throw new Error(`OpenAI API error ${response.status}: ${error}`);
  }

  const data = await response.json();
  return data.choices[0].message.content.trim();
}

async function callAnthropic(prompt, apiKey, model) {
  const response = await fetch(ANTHROPIC_URL, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "x-api-key": apiKey,
      "anthropic-version": ANTHROPIC_VERSION,
    },
    body: JSON.stringify({
      model,
      max_tokens: 1024,
      messages: [{ role: "user", content: prompt }],
    }),
  });

  if (!response.ok) {
    const error = await response.text();
    throw new Error(`Anthropic API error ${response.status}: ${error}`);
  }

  const data = await response.json();
  return data.content[0].text.trim();
}
