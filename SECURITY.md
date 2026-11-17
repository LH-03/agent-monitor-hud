# Security Policy

Please report security issues privately to the repository owner (GitHub private vulnerability
reporting, or a direct message) rather than in a public issue. Do not include real session logs,
prompts, credentials, API keys or databases in a report.

Task monitoring reads local metadata. Optional account requests are restricted to the official
DeepSeek balance and explicitly enabled Anthropic usage endpoints described in [PRIVACY.md](PRIVACY.md).
Credentials are kept in memory during requests, with redirects disabled. Changes that add other data transmission
require explicit documentation and maintainer review.
