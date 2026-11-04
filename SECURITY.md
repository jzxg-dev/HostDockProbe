# Security

If you find a security problem in hostdock-probe, please email **security@hostdock.dev** instead of opening a public issue. We will reply within a few days.

Especially interesting: any way for the probe service, or anyone between it and your server, to make the probe do something other than send metrics; anything the probe reads or sends that [docs/SECURITY-MODEL.md](docs/SECURITY-MODEL.md) says it does not.
