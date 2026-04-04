# Security

## API Key Storage

Narrator stores API keys locally at `~/.narrator/config.json` with file permissions `0600` (owner read/write only). Keys are **not encrypted** — they rely on OS file permissions for protection.

**Before sharing your machine or publishing screenshots**, ensure your API keys are not visible.

## Reporting Vulnerabilities

If you discover a security vulnerability, please report it privately by emailing the maintainer. Do not open a public issue.

## Best Practices

- Rotate your API keys periodically
- Use scoped/restricted API keys when possible (e.g., ElevenLabs keys with minimal permissions)
- Never commit `~/.narrator/config.json` to version control
