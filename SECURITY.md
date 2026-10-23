# Security policy

## Supported versions

Security fixes are released for the latest minor version. Update to the newest release before
reporting an issue.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub: **Security → Report a vulnerability** on
this repository (private vulnerability reporting), or email security@scalexy.com. Don't open a
public issue. You'll get an acknowledgement within 3 working days and a fix or mitigation plan
within 30 days for confirmed issues.

## Deployment notes

Scratchpad is a development tool that captures email, so treat its data as sensitive:

* By default the SMTP server and web interface listen on `127.0.0.1` only.
* When you bind to other interfaces (`--smtp-host 0.0.0.0`, `--web-host 0.0.0.0`), always set
  `--web-auth user:password` and put the web interface behind HTTPS (a reverse proxy) so passwords,
  including the vault's master password, are never sent in clear text.
* The link checker requests every link in captured emails, including private-network and
  `localhost` URLs (that's what makes it useful for local development). When strangers can send
  mail to an exposed SMTP server, turn the link checker off in *Settings*, or keep the server on a
  network where those requests are harmless.
* Share links and S3 public links are readable by anyone who has them. Prefer expiring private
  links for anything sensitive and delete shares you no longer need.
* Release artifacts are signed (Authenticode, Apple Developer ID, GPG) and carry Sigstore build
  provenance: `gh attestation verify <file> --repo scalexy/scratchpad`.
