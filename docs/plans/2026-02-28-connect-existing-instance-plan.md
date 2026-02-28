# Connect Existing Instance UX Improvement — Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Auto-discover local Docker instances on the Start page and replace the Docker-only "connect existing" dialog with an SSH remote connection flow.

**Architecture:** New `discover_local_instances` Tauri command scans Docker containers + `~/.clawpal/` data dirs and returns discovered instances. Start page renders unregistered discoveries as dashed-border cards with a "Connect" button. InstallHub's "connect" mode switches from Docker form to SSH form using existing `SshFormWidget`. `onEditSsh` in App.tsx gets wired to a real SSH edit dialog.

**Tech Stack:** Rust (Tauri commands), React/TypeScript, existing `SshFormWidget`, `InstanceCard`, `InstallHub`

---

### Task 1: Backend — `discover_local_instances` Tauri command

**Files:**
- Create: `src-tauri/src/commands/discover.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

**Step 1: Create `src-tauri/src/commands/discover.rs` with struct + scanning logic**

```rust
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredInstance {
    pub id: String,
    pub instance_type: String,
    pub label: String,
    pub home_path: String,
    pub source: String,
    pub container_name: Option<String>,
    pub already_registered: bool,
}

#[tauri::command]
pub async fn discover_local_instances() -> Result<Vec<DiscoveredInstance>, String> {
    let registered = clawpal_core::instance::InstanceRegistry::load()
        .map(|r| r.ids())
        .unwrap_or_default();
    let registered_set: HashSet<String> = registered.into_iter().collect();

    let mut results: Vec<DiscoveredInstance> = Vec::new();
    let mut seen_homes: HashSet<String> = HashSet::new();

    // 1. Docker container scan
    if let Ok(output) = Command::new("docker")
        .args(["ps", "--format", "{{json .}}"])
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                if let Ok(v) = serde_json::from_str::<Value>(line) {
                    let names = v.get("Names").and_then(Value::as_str).unwrap_or("");
                    let labels = v.get("Labels").and_then(Value::as_str).unwrap_or("");
                    let is_openclaw = names.contains("openclaw") || names.contains("clawpal")
                        || labels.contains("com.clawpal");
                    if !is_openclaw {
                        continue;
                    }
                    // Try to extract home from label
                    let home = labels
                        .split(',')
                        .find(|l| l.starts_with("com.clawpal.home="))
                        .and_then(|l| l.strip_prefix("com.clawpal.home="))
                        .map(String::from)
                        .unwrap_or_else(|| {
                            format!("~/.clawpal/{}", names.replace('/', "-"))
                        });
                    let id = format!("docker:{}", slug_from_name(names));
                    let already_registered = registered_set.contains(&id);
                    if seen_homes.insert(home.clone()) {
                        results.push(DiscoveredInstance {
                            id,
                            instance_type: "docker".to_string(),
                            label: format!("Docker ({})", names),
                            home_path: home,
                            source: "container".to_string(),
                            container_name: Some(names.to_string()),
                            already_registered,
                        });
                    }
                }
            }
        }
    }

    // 2. Data directory scan
    if let Some(home) = dirs::home_dir() {
        let clawpal_dir = home.join(".clawpal");
        if clawpal_dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&clawpal_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    let dir_name = path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    // Must look like a Docker instance directory
                    if !dir_name.starts_with("docker-") {
                        continue;
                    }
                    // Must have openclaw.json or docker-compose.yml
                    let has_config = path.join("openclaw.json").exists()
                        || path.join("docker-compose.yml").exists()
                        || path.join("docker-compose.yaml").exists();
                    if !has_config {
                        continue;
                    }
                    let home_path = format!("~/.clawpal/{}", dir_name);
                    if !seen_homes.insert(home_path.clone()) {
                        continue;
                    }
                    let id = format!("docker:{}", dir_name);
                    let already_registered = registered_set.contains(&id);
                    results.push(DiscoveredInstance {
                        id,
                        instance_type: "docker".to_string(),
                        label: dir_name.replace('-', " "),
                        home_path,
                        source: "data_dir".to_string(),
                        container_name: None,
                        already_registered,
                    });
                }
            }
        }
    }

    Ok(results)
}

fn slug_from_name(name: &str) -> String {
    let mut slug: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug.trim_matches('-').to_string()
}
```

**Step 2: Add module and register command**

In `src-tauri/src/commands/mod.rs`, add near the top:

```rust
pub mod discover;
```

In `src-tauri/src/lib.rs`, add `commands::discover::discover_local_instances` to the `invoke_handler` registration.

**Step 3: Expose `ids()` on `InstanceRegistry`**

In `clawpal-core/src/instance.rs`, add to `impl InstanceRegistry`:

```rust
pub fn ids(&self) -> Vec<String> {
    self.instances.keys().cloned().collect()
}
```

**Step 4: Run `cargo build -p clawpal-tauri`**

Expected: Compiles successfully.

**Step 5: Commit**

```bash
git add src-tauri/src/commands/discover.rs src-tauri/src/commands/mod.rs \
        src-tauri/src/lib.rs clawpal-core/src/instance.rs
git commit -m "feat: add discover_local_instances Tauri command"
```

---

### Task 2: Frontend — `DiscoveredInstance` type + API method

**Files:**
- Modify: `src/lib/types.ts:288` (after `RegisteredInstance`)
- Modify: `src/lib/api.ts`

**Step 1: Add `DiscoveredInstance` type**

In `src/lib/types.ts`, after the `RegisteredInstance` interface (line ~294), add:

```typescript
export interface DiscoveredInstance {
  id: string;
  instanceType: string;
  label: string;
  homePath: string;
  source: string;
  containerName?: string;
  alreadyRegistered: boolean;
}
```

**Step 2: Add API method**

In `src/lib/api.ts`, add inside the `api` object:

```typescript
discoverLocalInstances: (): Promise<DiscoveredInstance[]> =>
  invoke("discover_local_instances"),
```

Also add `DiscoveredInstance` to the import from `./types`.

**Step 3: Commit**

```bash
git add src/lib/types.ts src/lib/api.ts
git commit -m "feat: add DiscoveredInstance type and API method"
```

---

### Task 3: `InstanceCard` — discovered variant with dashed border

**Files:**
- Modify: `src/components/InstanceCard.tsx`

**Step 1: Add new props to `InstanceCardProps`**

At the end of the `InstanceCardProps` interface (line ~25), add:

```typescript
  discovered?: boolean;         // dashed border for unregistered discovered instances
  discoveredSource?: string;    // "container" | "data_dir" — shown as subtitle
  onConnect?: () => void;       // "Connect" button callback
```

**Step 2: Import `LinkIcon` from lucide-react**

Add `LinkIcon` to the lucide-react import:

```typescript
import { MonitorIcon, ContainerIcon, ServerIcon, EllipsisIcon, PencilIcon, Trash2Icon, RefreshCwIcon, LinkIcon } from "lucide-react";
```

**Step 3: Modify `Card` className for discovered variant**

In the `Card` component (line ~70), change the `className` to add dashed border when `discovered`:

```tsx
<Card
  className={cn(
    "cursor-pointer transition-all duration-300 group relative",
    "hover:shadow-[var(--shadow-warm-hover)]",
    opened && "border-primary/30",
    discovered && "border-dashed border-2 border-muted-foreground/30",
  )}
  onClick={discovered ? undefined : onClick}
>
```

**Step 4: Add "Connect" button at bottom of card for discovered instances**

After the health/agents section (line ~175, before the final `</CardContent>`), add:

```tsx
{discovered && onConnect && (
  <Button
    size="sm"
    className="w-full gap-1.5"
    onClick={(e) => { e.stopPropagation(); onConnect(); }}
  >
    <LinkIcon className="size-3.5" />
    {t("start.connect")}
  </Button>
)}
```

**Step 5: When `discovered`, hide the health/agents row (no health data for unregistered instances)**

Wrap the existing health/agents row (lines ~137-175) in a condition:

```tsx
{!discovered && (
  <div className="flex items-center gap-3 text-sm text-muted-foreground">
    {/* ... existing health/check/agents code ... */}
  </div>
)}
{discovered && discoveredSource && (
  <div className="text-xs text-muted-foreground">
    {discoveredSource === "container" ? t("start.fromContainer") : t("start.fromDataDir")}
  </div>
)}
```

**Step 6: Commit**

```bash
git add src/components/InstanceCard.tsx
git commit -m "feat: add discovered variant to InstanceCard"
```

---

### Task 4: StartPage — integrate auto-discovery

**Files:**
- Modify: `src/pages/StartPage.tsx`

**Step 1: Add imports and props**

Add `DiscoveredInstance` to imports from `@/lib/types`.

Add to `StartPageProps` interface:

```typescript
discoveredInstances: DiscoveredInstance[];
discoveringInstances: boolean;
onConnectDiscovered: (instance: DiscoveredInstance) => void;
```

Add the new props to the destructured function parameters.

**Step 2: Render discovered (unregistered) instances after the registered list**

After the registered instances map (line ~331, before the "+" card), add:

```tsx
{/* Discovered but unregistered instances */}
{discoveredInstances
  .filter((d) => !d.alreadyRegistered)
  .map((d) => (
    <InstanceCard
      key={`discovered-${d.id}`}
      id={d.id}
      label={d.label}
      type={d.instanceType === "docker" ? "docker" : "local"}
      healthy={null}
      agentCount={0}
      opened={false}
      onClick={() => {}}
      discovered
      discoveredSource={d.source}
      onConnect={() => onConnectDiscovered(d)}
    />
  ))}
```

**Step 3: Show scanning indicator when discoveringInstances is true**

Before the grid (after the welcome section), add:

```tsx
{discoveringInstances && (
  <div className="text-sm text-muted-foreground animate-pulse mb-2">
    {t("start.scanning")}
  </div>
)}
```

**Step 4: Commit**

```bash
git add src/pages/StartPage.tsx
git commit -m "feat: show discovered instances on Start page"
```

---

### Task 5: App.tsx — wire up discovery and connect logic

**Files:**
- Modify: `src/App.tsx`

**Step 1: Add state and import**

Import `DiscoveredInstance` from `@/lib/types`.

Add state near the existing instance state (around line ~200):

```typescript
const [discoveredInstances, setDiscoveredInstances] = useState<DiscoveredInstance[]>([]);
const [discoveringInstances, setDiscoveringInstances] = useState(false);
```

**Step 2: Add discovery function**

After `refreshRegisteredInstances` callback:

```typescript
const discoverInstances = useCallback(() => {
  setDiscoveringInstances(true);
  api.discoverLocalInstances()
    .then(setDiscoveredInstances)
    .catch(() => setDiscoveredInstances([]))
    .finally(() => setDiscoveringInstances(false));
}, []);
```

**Step 3: Trigger discovery on app load**

In the existing `useEffect` that calls `refreshHosts()` and `refreshRegisteredInstances()` on mount, add `discoverInstances()` call.

**Step 4: Add `handleConnectDiscovered` callback**

```typescript
const handleConnectDiscovered = useCallback(async (discovered: DiscoveredInstance) => {
  try {
    await withGuidance(
      () => api.connectDockerInstance(discovered.homePath, discovered.label, discovered.id),
      "connectDockerInstance",
      discovered.id,
      "docker_local",
    );
    refreshRegisteredInstances();
    discoverInstances();
    showToast(t("start.connected", { label: discovered.label }), "success");
  } catch (e) {
    showToast(e instanceof Error ? e.message : String(e), "error");
  }
}, [refreshRegisteredInstances, discoverInstances, showToast, t]);
```

**Step 5: Pass new props to StartPage**

At the `<StartPage>` render (line ~1071), add:

```tsx
discoveredInstances={discoveredInstances}
discoveringInstances={discoveringInstances}
onConnectDiscovered={handleConnectDiscovered}
```

**Step 6: Commit**

```bash
git add src/App.tsx
git commit -m "feat: wire auto-discovery to StartPage"
```

---

### Task 6: InstallHub — replace Docker connect with SSH connect

**Files:**
- Modify: `src/components/InstallHub.tsx`

**Step 1: Change "connect" tag text**

In `PRESET_TAGS` or the connect button (line ~580-586), update the tag key from "connect" to "connect_remote". The button label will use the updated i18n key `installChat.tag.connectRemote`.

**Step 2: Replace the `mode === "connect"` form section**

Replace the Docker-only form (lines ~507-541) with SSH connection flow:

```tsx
{mode === "connect" ? (
  <div className="space-y-4 py-2">
    <div className="text-sm text-muted-foreground">
      {t("installChat.connectRemoteDescription")}
    </div>
    <SshFormWidget
      invokeId="connect-ssh-form"
      onSubmit={(_invokeId, host) => handleSshConnectSubmit(host)}
      onCancel={() => setMode("idle")}
    />
    {runError && (
      <div className="text-sm text-destructive border border-destructive/30 rounded-md px-3 py-2 bg-destructive/5">
        {runError}
      </div>
    )}
  </div>
) : /* ... rest of modes ... */}
```

**Step 3: Add `handleSshConnectSubmit` function**

Inside the `InstallHub` component, add:

```typescript
const handleSshConnectSubmit = useCallback(async (host: SshHost) => {
  setConnectSubmitting(true);
  setRunError(null);
  try {
    // 1. Save SSH host config
    const saved = await api.upsertSshHost(host);
    // 2. Attempt SSH connection
    await api.sshConnect(saved.id);
    // 3. Check remote openclaw status
    await api.remoteGetInstanceStatus(saved.id);
    // 4. Success — close dialog
    const now = new Date().toISOString();
    onReady?.({
      id: `install-${Date.now()}`,
      method: "remote_ssh",
      state: "ready",
      current_step: null,
      logs: [],
      artifacts: {
        ssh_host_id: saved.id,
        ssh_host_label: saved.label,
      },
      created_at: now,
      updated_at: now,
    });
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    setRunError(message);
    showToast?.(message, "error");
  } finally {
    setConnectSubmitting(false);
  }
}, [onReady, showToast]);
```

**Step 4: Add `SshFormWidget` and `SshHost` imports**

`SshFormWidget` is already imported. Ensure `SshHost` is in the type import.

**Step 5: Remove old Docker-specific connect state**

Remove `connectPath` and `connectLabel` state variables, as well as `handleConnectSubmit`. Remove the old Docker connect form JSX. Keep `connectSubmitting` and `runError` as they're reused.

**Step 6: Update the connect mode footer**

Change the footer to just show a loading state if submitting:

```tsx
{mode === "connect" && connectSubmitting && (
  <DialogFooter>
    <div className="text-sm text-muted-foreground animate-pulse">
      {t("installChat.connecting")}
    </div>
  </DialogFooter>
)}
```

**Step 7: Commit**

```bash
git add src/components/InstallHub.tsx
git commit -m "feat: replace Docker connect form with SSH connection flow"
```

---

### Task 7: App.tsx — wire `onEditSsh` to real SSH edit dialog

**Files:**
- Modify: `src/App.tsx`

**Step 1: Add SSH edit dialog state**

Near the existing dialog states:

```typescript
const [sshEditOpen, setSshEditOpen] = useState(false);
const [editingSshHost, setEditingSshHost] = useState<SshHost | null>(null);
```

**Step 2: Create `handleEditSsh` callback**

```typescript
const handleEditSsh = useCallback((host: SshHost) => {
  setEditingSshHost(host);
  setSshEditOpen(true);
}, []);
```

**Step 3: Create `handleSshEditSave` callback**

```typescript
const handleSshEditSave = useCallback(async (host: SshHost) => {
  try {
    await withGuidance(
      () => api.upsertSshHost(host),
      "upsertSshHost",
      host.id,
      "remote_ssh",
    );
    refreshHosts();
    refreshRegisteredInstances();
    setSshEditOpen(false);
    showToast(t("instance.sshUpdated"), "success");
  } catch (e) {
    showToast(e instanceof Error ? e.message : String(e), "error");
  }
}, [refreshHosts, refreshRegisteredInstances, showToast, t]);
```

**Step 4: Wire `onEditSsh` prop on `StartPage`**

Replace `onEditSsh={() => {}}` (line ~1091) with:

```tsx
onEditSsh={handleEditSsh}
```

**Step 5: Add SSH edit dialog JSX**

After the existing dialogs in the JSX, add a Dialog that reuses `SshFormWidget`:

```tsx
<Dialog open={sshEditOpen} onOpenChange={setSshEditOpen}>
  <DialogContent>
    <DialogHeader>
      <DialogTitle>{t("instance.editSsh")}</DialogTitle>
    </DialogHeader>
    {editingSshHost && (
      <SshFormWidget
        invokeId="ssh-edit-form"
        defaults={editingSshHost}
        onSubmit={(_invokeId, host) => {
          handleSshEditSave({ ...host, id: editingSshHost.id });
        }}
        onCancel={() => setSshEditOpen(false)}
      />
    )}
  </DialogContent>
</Dialog>
```

Import `SshFormWidget` in App.tsx:

```typescript
import { SshFormWidget } from "@/components/SshFormWidget";
```

**Step 6: Commit**

```bash
git add src/App.tsx
git commit -m "feat: wire onEditSsh to real SSH edit dialog"
```

---

### Task 8: i18n — add translation keys

**Files:**
- Modify: `src/locales/zh.json`
- Modify: `src/locales/en.json`

**Step 1: Add new keys to `zh.json`**

```json
"start.scanning": "正在扫描本地实例...",
"start.connect": "连接",
"start.connected": "已连接 {{label}}",
"start.fromContainer": "来自 Docker 容器",
"start.fromDataDir": "来自数据目录",
"installChat.tag.connectRemote": "连接远程实例",
"installChat.connectRemoteTitle": "连接远程实例",
"installChat.connectRemoteDescription": "填写 SSH 连接信息以连接远程 OpenClaw 实例。",
"instance.editSsh": "编辑 SSH 连接",
"instance.sshUpdated": "SSH 配置已更新"
```

Also update existing key:
- `"installChat.tag.connect": "连接远程实例"` (was "连接已有实例")
- `"installChat.connectTitle": "连接远程实例"` (was "连接已有实例")

**Step 2: Add corresponding English keys to `en.json`**

```json
"start.scanning": "Scanning for local instances...",
"start.connect": "Connect",
"start.connected": "Connected {{label}}",
"start.fromContainer": "From Docker container",
"start.fromDataDir": "From data directory",
"installChat.tag.connectRemote": "Connect Remote Instance",
"installChat.connectRemoteTitle": "Connect Remote Instance",
"installChat.connectRemoteDescription": "Enter SSH connection details to connect to a remote OpenClaw instance.",
"instance.editSsh": "Edit SSH Connection",
"instance.sshUpdated": "SSH config updated"
```

Also update existing key:
- `"installChat.tag.connect": "Connect Remote Instance"`
- `"installChat.connectTitle": "Connect Remote Instance"`

**Step 3: Commit**

```bash
git add src/locales/zh.json src/locales/en.json
git commit -m "feat: add i18n keys for instance discovery and SSH connect"
```

---

### Task 9: Build verification + smoke test

**Step 1: Run Rust compilation**

```bash
cd /Users/zhixian/Codes/clawpal && cargo build -p clawpal-tauri 2>&1 | tail -5
```

Expected: Compiles successfully (no errors).

**Step 2: Run Rust tests**

```bash
cd /Users/zhixian/Codes/clawpal && cargo test --lib 2>&1 | tail -20
```

Expected: All existing tests pass + no regressions.

**Step 3: Run TypeScript type check**

```bash
cd /Users/zhixian/Codes/clawpal && npx tsc --noEmit 2>&1 | tail -20
```

Expected: No type errors.

**Step 4: Fix any compilation errors found**

Address any type mismatches, missing imports, or API signature issues.

**Step 5: Commit any fixes**

```bash
git add -A && git commit -m "fix: resolve build issues from instance discovery feature"
```
