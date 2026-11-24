# Proposal: README install section (human owns README.md)

```sh
curl -fsSL https://github.com/khairulalamruet-ai/DPanel/releases/latest/download/get.sh | sudo bash
```

Run as root on a new Ubuntu 22.04+/Debian 12+ server. Without options the panel is served under
`<ip>.sslip.io` and the first administrator is `owner` with the `super_admin` role (every
permission, like WHM's root). The password is printed once. Options: `--hostname`,
`--admin-email`, `--admin-username`, `--version`, `--no-agent`, `--check`
(`curl ... | sudo bash -s -- --hostname panel.example.com`).

Note: the username `root` stays reserved on purpose (it cannot be taken by an account).
