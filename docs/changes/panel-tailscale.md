### Added
- `kora panel --tailscale`: open Kora Panel from your own devices over Tailscale in one step. It makes the Tailscale account you are connected from (or the computer's owner, or `--admin`) the panel's admin, keeps the background panel listening on this computer and on its Tailscale addresses (never the public one), and prints the address to open. You sign in with your Tailscale account: no link, token or SSH tunnel. With `ufw` on it allows the port on the Tailscale interface only.
- `kora panel tailscale allow|remove|list|off` to manage who may sign in with Tailscale, without opening the panel.
- On a server with no screen and Tailscale running, `kora panel --setup` (run by the install script) offers to set this up. `KORA_TAILSCALE=1` says yes in advance, `KORA_TAILSCALE=0` never asks.
- `kora panel --new-token` makes a new sign-in token for the background panel (links shown before stop working).

### Fixed
- `kora panel --bind <address> --remote` no longer prints the 127.0.0.1 link of the panel already running in the background and exits: it starts the panel on that address.

### Security
- The panel refuses to listen on a public address or on every interface (`0.0.0.0`, `::`) unless `--allow-public` is given: it is plain HTTP.
