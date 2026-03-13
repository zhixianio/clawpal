# Local Docker OpenClaw Debug Environment

## Goal

Use a disposable Ubuntu container as an isolated OpenClaw target for ClawPal recipe testing.

This keeps recipe validation away from your host `~/.openclaw` and away from production VPS instances.

## What this environment contains

- A fresh `ubuntu:22.04` container
- SSH exposed on `127.0.0.1:2299`
- OpenClaw installed via the official installer
- A minimal OpenClaw config that ClawPal can discover
- One baseline agent: `main`
- One baseline model: `openai/gpt-4o`
- One Discord fixture:
  - `guild-recipe-lab`
  - `channel-general`
  - `channel-support`

Recommended remote instance settings inside ClawPal:

- Label: `Local Remote SSH`
- Host: `127.0.0.1`
- Port: `2299`
- Username: `root`
- Password: `clawpal-recipe-pass`

## Important rule

Do not keep ClawPal connected to the container while OpenClaw is still being installed or seeded.

ClawPal may probe the remote host, detect that `openclaw` is missing, and trigger overlapping auto-install flows. That can leave `apt`/`dpkg` locked inside the container and make the bootstrap flaky.

Safe sequence:

1. Build the container.
2. Install and seed OpenClaw.
3. Verify the remote CLI works over SSH.
4. Only then launch `bun run dev:tauri` and connect ClawPal.

## Rebuild from scratch

### 1. Remove any previous test containers

```bash
docker rm -f clawpal-recipe-test-ubuntu-openclaw sweet_jang
```

`sweet_jang` was a previously reused image/container in local debugging. Remove it too so the new environment starts from a clean Ubuntu base.

### 2. Start a fresh Ubuntu container

```bash
docker run -d \
  --name clawpal-recipe-test-ubuntu-openclaw \
  -p 2299:22 \
  -p 18799:18789 \
  ubuntu:22.04 \
  sleep infinity
```

### 3. Install SSH and base packages

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw apt-get update
docker exec clawpal-recipe-test-ubuntu-openclaw apt-get install -y \
  openssh-server curl ca-certificates git xz-utils jq
```

### 4. Enable root password login for local debugging

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw sh -lc '
  echo "root:clawpal-recipe-pass" | chpasswd &&
  mkdir -p /run/sshd &&
  sed -i "s/^#\\?PermitRootLogin .*/PermitRootLogin yes/" /etc/ssh/sshd_config &&
  sed -i "s/^#\\?PasswordAuthentication .*/PasswordAuthentication yes/" /etc/ssh/sshd_config &&
  /usr/sbin/sshd
'
```

### 5. Install OpenClaw

Use the official installer:

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw sh -lc '
  curl -fsSL --proto "=https" --tlsv1.2 https://openclaw.ai/install.sh | \
  bash -s -- --no-prompt --no-onboard
'
```

Expected check:

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw openclaw --version
```

## Seed the minimal test fixture

### 6. Bootstrap the config file with the OpenClaw CLI

Create `~/.openclaw/openclaw.json` through OpenClaw itself:

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw \
  openclaw config set gateway.port 18789 --strict-json
```

Seed a minimal provider catalog:

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw sh -lc '
  openclaw config set models.providers \
    "{\"openai\":{\"baseUrl\":\"https://api.openai.com/v1\",\"models\":[{\"id\":\"gpt-4o\",\"name\":\"GPT-4o\"}]}}" \
    --strict-json
'
```

Set the default model:

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw \
  openclaw models set openai/gpt-4o
```

### 7. Seed the default agent identity with the OpenClaw CLI

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw \
  openclaw agents set-identity \
  --agent main \
  --name "Main Agent" \
  --emoji "🤖" \
  --json
```

### 8. Seed Discord test channels with the OpenClaw CLI

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw sh -lc '
  openclaw config set channels.discord \
    "{\"guilds\":{\"guild-recipe-lab\":{\"channels\":{\"channel-general\":{\"systemPrompt\":\"\"},\"channel-support\":{\"systemPrompt\":\"\"}}}}}" \
    --strict-json
'
```

### 9. Seed a test auth profile

Current boundary: this part is still a controlled file seed, not a pure OpenClaw CLI flow.

Reason:

- `openclaw models auth paste-token` is interactive
- the current local recipe/debug flow needs a non-interactive baseline credential

Until OpenClaw exposes a stable non-interactive auth seed command, use:

```bash
docker exec clawpal-recipe-test-ubuntu-openclaw sh -lc '
  mkdir -p /root/.openclaw/agents/main/agent &&
  cat > /root/.openclaw/agents/main/agent/auth-profiles.json <<\"EOF\"
{"version":1,"profiles":{"openai:default":{"type":"api_key","provider":"openai","secretRef":{"source":"env","id":"OPENAI_API_KEY"}}}}
EOF
  printf "export OPENAI_API_KEY=test-openai-key\n" >> /root/.profile
  printf "export OPENAI_API_KEY=test-openai-key\n" >> /root/.bash_profile
'
```

This is the one intentional exception to the `OpenClaw-first` rule for this local debug fixture.

## Verify the container before opening ClawPal

### 10. Verify over SSH

Agent list:

```bash
expect -c 'set timeout 20; \
  spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2299 root@127.0.0.1 openclaw agents list --json; \
  expect "password:"; \
  send "clawpal-recipe-pass\r"; \
  expect eof'
```

Discord fixture:

```bash
expect -c 'set timeout 20; \
  spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -p 2299 root@127.0.0.1 openclaw config get channels.discord --json; \
  expect "password:"; \
  send "clawpal-recipe-pass\r"; \
  expect eof'
```

You should see:

- `main` as the default agent
- `openai/gpt-4o` as the model
- `guild-recipe-lab`
- `channel-general`
- `channel-support`

## Use it inside ClawPal

Once the checks above pass:

1. Start ClawPal:
   ```bash
   bun run dev:tauri
   ```
2. Add or reuse the remote SSH instance:
   - Host: `127.0.0.1`
   - Port: `2299`
   - User: `root`
   - Password: `clawpal-recipe-pass`
3. Open `Recipes`
4. Use the bundled recipes against this isolated target

## What this fixture is good for

- `Dedicated Agent`
- `Agent Persona Pack`
- `Channel Persona Pack`
- Review/Execute/Done UX
- remote discovery for:
  - agents
  - guilds/channels
  - remote config snapshots
  - recipe runtime writes

## Troubleshooting

### Agent or guild dropdowns are empty

Check these two commands first:

```bash
ssh -p 2299 root@127.0.0.1 openclaw agents list --json
ssh -p 2299 root@127.0.0.1 openclaw config get channels.discord --json
```

If either fails, fix the container before debugging the UI.

### OpenClaw installer hangs or apt is locked

Likely cause: ClawPal connected too early and triggered an overlapping auto-install attempt.

Recovery:

1. Stop ClawPal.
2. Stop `sshd` in the container.
3. Kill leftover installer processes.
4. Run `dpkg --configure -a`.
5. Retry the OpenClaw install once.

### Docker daemon itself becomes unhealthy

If `docker version` hangs or returns socket errors:

1. Restart Docker Desktop.
2. Confirm `docker version` works.
3. Rebuild the container from scratch.

## Maintenance note

Keep this local debug fixture aligned with the Docker E2E path in:

- [recipe_docker_e2e.rs](../../src-tauri/tests/recipe_docker_e2e.rs)

If the required OpenClaw schema changes, update both:

- the local debug fixture in this document
- the E2E fixture and assertions
