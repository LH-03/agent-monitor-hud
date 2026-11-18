# Security Policy

Please report security issues privately to the repository owner (GitHub private vulnerability
reporting, or a direct message) rather than in a public issue. Do not include real session logs,
prompts, credentials, API keys or databases in a report.

Task monitoring reads local metadata. Account readings come from the local Codex client and the
optional DeepSeek balance endpoint described in [PRIVACY.md](PRIVACY.md). The balance client
disables redirects and keeps the key in memory during the request.
