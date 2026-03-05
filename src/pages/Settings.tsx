import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { FormEvent } from "react";
import { useTranslation } from "react-i18next";
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { getVersion } from "@tauri-apps/api/app";
import { toast } from "sonner";
import { hasGuidanceEmitted, useApi } from "@/lib/use-api";
import { isAlreadyExplainedGuidanceError } from "@/lib/guidance";
import { useTheme } from "@/lib/use-theme";
import { useFont } from "@/lib/use-font";
import type { UiFont } from "@/lib/use-font";
import { profileToModelValue } from "@/lib/model-value";
import { resolveProfileCredentialView } from "@/lib/profile-credential";
import type {
  ModelCatalogProvider,
  ModelProfile,
  ProviderAuthSuggestion,
  ResolvedApiKey,
  ZeroclawRuntimeTarget,
  ZeroclawUsageStats,
} from "@/lib/types";
import { Card, CardHeader, CardTitle, CardContent } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";

type ProfileForm = {
  id: string;
  provider: string;
  model: string;
  authRef: string;
  apiKey: string;
  useCustomUrl: boolean;
  baseUrl: string;
  enabled: boolean;
};

type CredentialSource = "oauth" | "env" | "manual";

const MODEL_CATALOG_CACHE_TTL_MS = 5 * 60_000;
const ENABLE_PROFILE_TEST_BUTTON = true;
let modelCatalogCache: { value: ModelCatalogProvider[]; expiresAt: number } | null = null;
let profilesExtractedOnce = false;
const PROVIDER_FALLBACK_OPTIONS = [
  "openai",
  "openai-codex",
  "anthropic",
  "openrouter",
  "ollama",
  "lmstudio",
  "localai",
  "vllm",
];

function emptyForm(): ProfileForm {
  return {
    id: "",
    provider: "",
    model: "",
    authRef: "",
    apiKey: "",
    useCustomUrl: false,
    baseUrl: "",
    enabled: true,
  };
}

function normalizeOauthProvider(provider: string): string {
  const lower = provider.trim().toLowerCase();
  if (lower === "openai_codex" || lower === "github-copilot" || lower === "copilot") {
    return "openai-codex";
  }
  return lower;
}

function providerUsesOAuthAuth(provider: string): boolean {
  return normalizeOauthProvider(provider) === "openai-codex";
}

function defaultOauthAuthRef(provider: string): string {
  const normalized = normalizeOauthProvider(provider);
  if (normalized === "openai-codex") {
    return "openai-codex:default";
  }
  return "";
}

function oauthProfileNameFromAuthRef(authRef: string): string {
  const trimmed = authRef.trim();
  if (!trimmed) return "default";
  const idx = trimmed.indexOf(":");
  if (idx < 0) return "default";
  const profile = trimmed.slice(idx + 1).trim();
  return profile || "default";
}

function isEnvVarLikeAuthRef(authRef: string): boolean {
  return /^[A-Za-z_][A-Za-z0-9_]*$/.test(authRef.trim());
}

function defaultEnvAuthRef(provider: string): string {
  const normalized = normalizeOauthProvider(provider);
  if (!normalized) return "";
  if (normalized === "openai-codex") {
    return "OPENAI_CODEX_TOKEN";
  }
  const providerEnv = normalized
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
    .toUpperCase();
  return providerEnv ? `${providerEnv}_API_KEY` : "";
}

function inferCredentialSource(provider: string, authRef: string): CredentialSource {
  const trimmed = authRef.trim();
  if (!trimmed) {
    return providerUsesOAuthAuth(provider) ? "oauth" : "manual";
  }
  if (providerUsesOAuthAuth(provider) && trimmed.toLowerCase().startsWith("openai-codex:")) {
    return "oauth";
  }
  return "env";
}

function providerSupportsOptionalApiKey(provider: string): boolean {
  if (providerUsesOAuthAuth(provider)) {
    return true;
  }
  const lower = provider.trim().toLowerCase();
  return [
    "ollama",
    "lmstudio",
    "lm-studio",
    "localai",
    "vllm",
    "llamacpp",
    "llama.cpp",
  ].includes(lower);
}

function AutocompleteField({
  value,
  onChange,
  onFocus,
  options,
  placeholder,
}: {
  value: string;
  onChange: (val: string) => void;
  onFocus?: () => void;
  options: { value: string; label: string }[];
  placeholder: string;
}) {
  const [open, setOpen] = useState(false);
  const wrapperRef = useRef<HTMLDivElement>(null);

  const filtered = options.filter(
    (o) =>
      !value ||
      o.value.toLowerCase().includes(value.toLowerCase()) ||
      o.label.toLowerCase().includes(value.toLowerCase()),
  );

  useEffect(() => {
    function handleClickOutside(e: MouseEvent) {
      if (wrapperRef.current && !wrapperRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  return (
    <div ref={wrapperRef} className="relative">
      <Input
        placeholder={placeholder}
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          setOpen(true);
        }}
        onFocus={() => {
          setOpen(true);
          onFocus?.();
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") setOpen(false);
        }}
      />
      {open && filtered.length > 0 && (
        <div className="absolute z-50 w-full mt-1 bg-popover border border-border rounded-md shadow-md max-h-[200px] overflow-y-auto">
          {filtered.map((option) => (
            <div
              key={option.value}
              className="px-3 py-1.5 text-sm cursor-pointer hover:bg-accent hover:text-accent-foreground"
              onMouseDown={(e) => {
                e.preventDefault();
                onChange(option.value);
                setOpen(false);
              }}
            >
              {option.label}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export function Settings({
  onDataChange,
  hasAppUpdate,
  onAppUpdateSeen,
  globalMode = false,
  section = "all",
  onOpenDoctor,
  onNavigateToProfiles,
}: {
  onDataChange?: () => void;
  hasAppUpdate?: boolean;
  onAppUpdateSeen?: () => void;
  globalMode?: boolean;
  section?: "all" | "profiles" | "preferences";
  onOpenDoctor?: () => void;
  onNavigateToProfiles?: () => void;
}) {
  const { t, i18n } = useTranslation();
  const ua = useApi();
  const { theme, setTheme } = useTheme();
  const { font, setFont } = useFont();
  const [profiles, setProfiles] = useState<ModelProfile[] | null>(null);
  const [catalog, setCatalog] = useState<ModelCatalogProvider[]>([]);
  const [apiKeys, setApiKeys] = useState<ResolvedApiKey[]>([]);
  const [form, setForm] = useState<ProfileForm>(emptyForm());
  const [credentialSource, setCredentialSource] = useState<CredentialSource>("manual");
  const [profileDialogOpen, setProfileDialogOpen] = useState(false);
  const [message, setMessage] = useState("");
  const [authSuggestion, setAuthSuggestion] = useState<ProviderAuthSuggestion | null>(null);
  const [oauthAuthorizeUrl, setOauthAuthorizeUrl] = useState("");
  const [oauthRedirectInput, setOauthRedirectInput] = useState("");
  const [oauthStarting, setOauthStarting] = useState(false);
  const [oauthCompleting, setOauthCompleting] = useState(false);
  const [testingProfileId, setTestingProfileId] = useState<string | null>(null);
  const [zeroclawModel, setZeroclawModel] = useState("");

  const [zeroclawUsage, setZeroclawUsage] = useState<ZeroclawUsageStats | null>(null);
  const [zeroclawUsageLoading, setZeroclawUsageLoading] = useState(true);
  const [zeroclawTarget, setZeroclawTarget] = useState<ZeroclawRuntimeTarget | null>(null);
  const [zeroclawTargetLoading, setZeroclawTargetLoading] = useState(true);
  const [showZeroclawDoctorUi, setShowZeroclawDoctorUi] = useState(false);
  const [showRescueBotUi, setShowRescueBotUi] = useState(false);
  const [showSshTransferSpeedUi, setShowSshTransferSpeedUi] = useState(false);
  const zeroclawPrefsLoadedRef = useRef(false);
  const zeroclawLastSavedRef = useRef("");

  const [catalogRefreshed, setCatalogRefreshed] = useState(false);

  // ClawPal app version & self-update
  const [appVersion, setAppVersion] = useState<string>("");
  const [appUpdate, setAppUpdate] = useState<{ version: string; body?: string } | null>(null);
  const [appUpdateChecking, setAppUpdateChecking] = useState(false);
  const [appUpdating, setAppUpdating] = useState(false);
  const [appUpdateProgress, setAppUpdateProgress] = useState<number | null>(null);

  useEffect(() => {
    getVersion().then(setAppVersion).catch(() => {});
  }, []);

  const handleCheckForUpdates = useCallback(async () => {
    setAppUpdateChecking(true);
    setAppUpdate(null);
    try {
      const update = await check();
      if (update) {
        setAppUpdate({ version: update.version, body: update.body });
      }
    } catch (e) {
      console.error("Update check failed:", e);
    } finally {
      setAppUpdateChecking(false);
    }
  }, []);

  const handleAppUpdate = useCallback(async () => {
    setAppUpdating(true);
    setAppUpdateProgress(0);
    try {
      const update = await check();
      if (!update) return;
      let totalBytes = 0;
      let downloadedBytes = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && event.data.contentLength) {
          totalBytes = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          if (totalBytes > 0) {
            setAppUpdateProgress(Math.round((downloadedBytes / totalBytes) * 100));
          }
        } else if (event.event === "Finished") {
          setAppUpdateProgress(100);
        }
      });
      await relaunch();
    } catch (e) {
      console.error("App update failed:", e);
      setAppUpdating(false);
      setAppUpdateProgress(null);
    }
  }, []);

  // Auto-trigger update check when navigated to from red dot
  useEffect(() => {
    if (hasAppUpdate) {
      handleCheckForUpdates();
      onAppUpdateSeen?.();
    }
  }, [hasAppUpdate, handleCheckForUpdates, onAppUpdateSeen]);

  // Extract profiles from config on first load
  useEffect(() => {
    if (profilesExtractedOnce) return;
    profilesExtractedOnce = true;
    ua.extractModelProfilesFromConfig()
      .catch((e) => {
        profilesExtractedOnce = false;
        console.error("Failed to extract profiles:", e);
      });
  }, [ua]);

  const refreshProfiles = () => {
    const withTimeout = <T,>(promise: Promise<T>, timeoutMs: number, fallback: T): Promise<T> =>
      Promise.race([
        promise,
        new Promise<T>((resolve) => setTimeout(() => resolve(fallback), timeoutMs)),
      ]);

    withTimeout(ua.listModelProfiles(), 8000, [])
      .then(setProfiles)
      .catch((e) => {
        console.error("Failed to load profiles:", e);
        setProfiles([]);
      });
    withTimeout(ua.resolveApiKeys(), 8000, [])
      .then(setApiKeys)
      .catch((e) => {
        console.error("Failed to resolve API keys:", e);
        setApiKeys([]);
      });
  };

  useEffect(refreshProfiles, [ua]);

  useEffect(() => {
    ua.getAppPreferences()
      .then((prefs) => {
        const value = prefs.zeroclawModel || "";
        setZeroclawModel(value);
        zeroclawLastSavedRef.current = value.trim();
        zeroclawPrefsLoadedRef.current = true;

        const nextShowZeroclawUi = Boolean(prefs.showZeroclawDoctorUi);
        setShowZeroclawDoctorUi(nextShowZeroclawUi);
        setShowRescueBotUi(Boolean(prefs.showRescueBotUi));
        setShowSshTransferSpeedUi(Boolean(prefs.showSshTransferSpeedUi));
      })
      .catch((e) => console.error("Failed to load app preferences:", e));
  }, [ua]);

  useEffect(() => {
    let cancelled = false;
    let firstLoad = true;
    const loadStats = () => {
      // Only show loading spinners on the initial fetch to avoid flickering.
      if (firstLoad) {
        setZeroclawUsageLoading(true);
        setZeroclawTargetLoading(true);
      }
      ua.getZeroclawUsageStats()
        .then((stats) => {
          if (!cancelled) setZeroclawUsage(stats);
        })
        .catch(() => {
          if (!cancelled) setZeroclawUsage(null);
        })
        .finally(() => {
          if (!cancelled) setZeroclawUsageLoading(false);
        });
      ua.getZeroclawRuntimeTarget()
        .then((target) => {
          if (!cancelled) setZeroclawTarget(target);
        })
        .catch(() => {
          if (!cancelled) setZeroclawTarget(null);
        })
        .finally(() => {
          if (!cancelled) setZeroclawTargetLoading(false);
          firstLoad = false;
        });
    };
    loadStats();
    const timer = window.setInterval(loadStats, 4_000);
    return () => {
      cancelled = true;
      window.clearInterval(timer);
    };
  }, [ua]);

  const zeroclawTargetText = useMemo(() => {
    if (zeroclawTargetLoading) return t("settings.zeroclawEffectiveModelLoading");
    const provider = zeroclawTarget?.provider?.trim();
    if (!provider) return t("settings.zeroclawEffectiveModelUnavailable");
    const model = zeroclawTarget?.model?.trim();
    const modelLabel = model ? `${provider}/${model}` : provider;
    if (zeroclawTarget?.source === "preferred") {
      return t("settings.zeroclawEffectiveModelPreferred", { model: modelLabel });
    }
    return t("settings.zeroclawEffectiveModelAuto", { model: modelLabel });
  }, [zeroclawTargetLoading, zeroclawTarget, t]);

  // Load catalog on mount
  useEffect(() => {
    const now = Date.now();
    if (modelCatalogCache && modelCatalogCache.expiresAt > now) {
      setCatalog(modelCatalogCache.value);
      setCatalogRefreshed(true);
      return;
    }
    setCatalogRefreshed(false);
    ua.refreshModelCatalog()
      .then((fresh) => {
        setCatalog(fresh);
        modelCatalogCache = {
          value: fresh,
          expiresAt: Date.now() + MODEL_CATALOG_CACHE_TTL_MS,
        };
      })
      .catch((e) => console.error("Failed to load model catalog:", e));
  }, [ua]);

  // Refresh catalog from CLI when user focuses provider/model input
  const ensureCatalog = () => {
    if (catalogRefreshed) return;
    setCatalogRefreshed(true);
    ua.refreshModelCatalog().then((fresh) => {
      if (fresh.length > 0) setCatalog(fresh);
      modelCatalogCache = {
        value: fresh,
        expiresAt: Date.now() + MODEL_CATALOG_CACHE_TTL_MS,
      };
    }).catch((e) => console.error("Failed to refresh model catalog:", e));
  };

  const resolvedCredentialMap = useMemo(() => {
    const map = new Map<string, ResolvedApiKey>();
    for (const entry of apiKeys) {
      map.set(entry.profileId, entry);
    }
    return map;
  }, [apiKeys]);

  // Check for existing auth when provider changes
  useEffect(() => {
    if (form.id || !form.provider.trim()) {
      setAuthSuggestion(null);
      return;
    }
    if (ua.isRemote) {
      // For remote: infer from existing profiles
      const existing = (profiles || []).find(
        (p) => {
          if (p.provider !== form.provider) return false;
          const credential = resolveProfileCredentialView(p, resolvedCredentialMap.get(p.id));
          return credential.resolved;
        }
      );
      if (existing) {
        const credential = resolveProfileCredentialView(
          existing,
          resolvedCredentialMap.get(existing.id),
        );
        setAuthSuggestion({
          hasKey: true,
          source: `existing profile (${existing.provider}/${existing.model})`,
          authRef: credential.authRef || existing.authRef || "",
        });
      } else {
        setAuthSuggestion(null);
      }
    } else {
      ua.resolveProviderAuth(form.provider)
        .then(setAuthSuggestion)
        .catch((e) => { console.error("Failed to resolve provider auth:", e); setAuthSuggestion(null); });
    }
  }, [form.provider, form.id, ua, profiles, resolvedCredentialMap]);

  useEffect(() => {
    if (!providerUsesOAuthAuth(form.provider)) {
      setOauthAuthorizeUrl("");
      setOauthRedirectInput("");
    }
  }, [form.provider]);

  useEffect(() => {
    if (!providerUsesOAuthAuth(form.provider) && credentialSource === "oauth") {
      setCredentialSource("env");
    }
  }, [form.provider, credentialSource]);

  const modelCandidates = useMemo(() => {
    const found = catalog.find((c) => c.provider === form.provider);
    return found?.models || [];
  }, [catalog, form.provider]);

  const providerCandidates = useMemo(() => {
    const set = new Set<string>();
    for (const provider of PROVIDER_FALLBACK_OPTIONS) {
      if (provider.trim()) set.add(provider);
    }
    for (const item of catalog) {
      const provider = item.provider.trim();
      if (provider) set.add(provider);
    }
    for (const profile of profiles || []) {
      const provider = profile.provider.trim();
      if (provider) set.add(provider);
    }
    return Array.from(set).sort((a, b) => a.localeCompare(b));
  }, [catalog, profiles]);

  const zeroclawModelCandidates = useMemo(() => {
    const fromProfiles = (profiles || [])
      .filter((profile) => profile.enabled)
      .filter((profile) => profile.provider.trim())
      .map((profile) => profileToModelValue(profile));
    return Array.from(new Set(fromProfiles)).sort((a, b) => a.localeCompare(b));
  }, [profiles]);

  useEffect(() => {
    if (!zeroclawPrefsLoadedRef.current) return;
    // Skip validation until profiles have loaded; an empty candidate list
    // before that point would incorrectly clear the persisted selection.
    if (profiles === null) return;
    const current = zeroclawModel.trim();
    if (!current) return;
    const exists = zeroclawModelCandidates.some(
      (candidate) => candidate.toLowerCase() === current.toLowerCase(),
    );
    if (!exists) {
      setZeroclawModel("");
    }
  }, [zeroclawModel, zeroclawModelCandidates, profiles]);

  const saveProfile = async (authRefOverride?: string): Promise<boolean> => {
    if (!form.provider || !form.model) {
      setMessage(t('settings.providerModelRequired'));
      return false;
    }
    const apiKeyOptional = form.useCustomUrl || providerSupportsOptionalApiKey(form.provider);
    const oauthSource = credentialSource === "oauth" && providerUsesOAuthAuth(form.provider);
    const envSource = credentialSource === "env";
    const manualSource = credentialSource === "manual";
    if (!ua.isRemote && manualSource && !form.apiKey && !form.id && !apiKeyOptional) {
      setMessage(t('settings.apiKeyRequired'));
      return false;
    }
    const overrideAuthRef = (authRefOverride || "").trim();
    const explicitAuthRef = form.authRef.trim();
    const oauthFallbackAuthRef = defaultOauthAuthRef(form.provider);
    const resolvedAuthRef = oauthSource
      ? (overrideAuthRef || explicitAuthRef || oauthFallbackAuthRef)
      : envSource
        ? (
          overrideAuthRef
          || explicitAuthRef
          || ((!form.apiKey && authSuggestion?.authRef) ? authSuggestion.authRef : "")
        )
        : "";
    const profileData: ModelProfile = {
      id: form.id || "",
      name: `${form.provider}/${form.model}`,
      provider: form.provider,
      model: form.model,
      authRef: resolvedAuthRef,
      apiKey: form.apiKey || undefined,
      baseUrl: form.useCustomUrl && form.baseUrl ? form.baseUrl : undefined,
      enabled: form.enabled,
    };
    try {
      await ua.upsertModelProfile(profileData);
      setMessage(t('settings.profileSaved'));
      setForm(emptyForm());
      setProfileDialogOpen(false);
      setOauthAuthorizeUrl("");
      setOauthRedirectInput("");
      refreshProfiles();
      onDataChange?.();
      return true;
    } catch (e) {
      setMessage(t('settings.saveFailed', { error: String(e) }));
      return false;
    }
  };

  const upsert = (event: FormEvent) => {
    event.preventDefault();
    void saveProfile();
  };

  const startOauthLogin = async () => {
    if (!providerUsesOAuthAuth(form.provider)) return;
    const provider = normalizeOauthProvider(form.provider);
    const authRef = form.authRef.trim() || defaultOauthAuthRef(provider);
    const profile = oauthProfileNameFromAuthRef(authRef);
    setOauthStarting(true);
    try {
      const result = await ua.startZeroclawOauthLogin(provider, profile, ua.instanceId);
      setForm((prev) => ({ ...prev, authRef: result.authRef || prev.authRef || authRef }));
      setOauthAuthorizeUrl(result.authorizeUrl);
      setMessage(t("settings.oauthStartSuccess"));
    } catch (e) {
      setMessage(t("settings.oauthStartFailed", { error: String(e) }));
    } finally {
      setOauthStarting(false);
    }
  };

  const completeOauthAndSave = async () => {
    if (!providerUsesOAuthAuth(form.provider)) return;
    const provider = normalizeOauthProvider(form.provider);
    const authRef = form.authRef.trim() || defaultOauthAuthRef(provider);
    const profile = oauthProfileNameFromAuthRef(authRef);
    const redirectInput = oauthRedirectInput.trim();
    if (!redirectInput) {
      setMessage(t("settings.oauthRedirectRequired"));
      return;
    }
    setOauthCompleting(true);
    try {
      const result = await ua.completeZeroclawOauthLogin(
        provider,
        redirectInput,
        profile,
        ua.instanceId,
      );
      const resolvedAuthRef = result.authRef || authRef;
      setForm((prev) => ({ ...prev, authRef: resolvedAuthRef }));
      await saveProfile(resolvedAuthRef);
    } catch (e) {
      setMessage(t("settings.oauthCompleteFailed", { error: String(e) }));
    } finally {
      setOauthCompleting(false);
    }
  };

  const editProfile = (profile: ModelProfile) => {
    setCredentialSource(inferCredentialSource(profile.provider, profile.authRef || ""));
    setForm({
      id: profile.id,
      provider: profile.provider,
      model: profile.model,
      authRef: profile.authRef || "",
      apiKey: "",
      useCustomUrl: !!profile.baseUrl,
      baseUrl: profile.baseUrl || "",
      enabled: profile.enabled,
    });
    setOauthAuthorizeUrl("");
    setOauthRedirectInput("");
    setOauthStarting(false);
    setOauthCompleting(false);
    setProfileDialogOpen(true);
  };

  const openAddProfile = () => {
    setCredentialSource("manual");
    setForm(emptyForm());
    setOauthAuthorizeUrl("");
    setOauthRedirectInput("");
    setOauthStarting(false);
    setOauthCompleting(false);
    setProfileDialogOpen(true);
  };

  const deleteProfile = (id: string) => {
    ua.deleteModelProfile(id)
      .then(() => {
        setMessage(t('settings.profileDeleted'));
        if (form.id === id) {
          setForm(emptyForm());
        }
        refreshProfiles();
        onDataChange?.();
      })
      .catch((e) => setMessage(t('settings.deleteFailed', { error: String(e) })));
  };

  const toggleProfileEnabled = (profile: ModelProfile) => {
    const nextEnabled = !profile.enabled;
    ua.upsertModelProfile({
      ...profile,
      enabled: nextEnabled,
    })
      .then(() => {
        const message = nextEnabled
          ? t('settings.profileEnabledMessage', { name: `${profile.provider}/${profile.model}` })
          : t('settings.profileDisabledMessage', { name: `${profile.provider}/${profile.model}` });
        toast.success(message);
        refreshProfiles();
        onDataChange?.();
      })
      .catch((e) => {
        const errorText = e instanceof Error ? e.message : String(e);
        toast.error(t('settings.saveFailed', { error: errorText }));
      });
  };

  const testProfile = async (profile: ModelProfile) => {
    if (!profile.enabled) {
      toast.error(t('settings.testProfileDisabled'));
      return;
    }
    setTestingProfileId(profile.id);
    try {
      await ua.testModelProfile(profile.id);

      toast.success(
        t('settings.testProfileSuccess', {
          name: `${profile.provider}/${profile.model}`,
        }),
      );
    } catch (e) {
      const errorText = e instanceof Error ? e.message : String(e);
      if (hasGuidanceEmitted(e) || isAlreadyExplainedGuidanceError(errorText)) {
        if (onOpenDoctor) {
          toast.error(
            <div className="space-y-2">
              <p>{t('settings.testProfileFailed', { error: t('home.fixInDoctor') })}</p>
              <div className="flex flex-wrap gap-2">
                <Button
                  type="button"
                  size="sm"
                  variant="outline"
                  onClick={() => {
                    onOpenDoctor();
                  }}
                >
                  {t("home.fixInDoctor")}
                </Button>
              </div>
            </div>
          );
        }
        return;
      }
      toast.error(
        <div className="space-y-2">
          <p>{t('settings.testProfileFailed', { error: errorText })}</p>
          <div className="flex flex-wrap gap-2">
          {onOpenDoctor && (
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => {
                onOpenDoctor();
              }}
            >
              {t("home.fixInDoctor")}
            </Button>
          )}
          </div>
        </div>
      );
    } finally {
      setTestingProfileId(null);
    }
  };

  const showProfiles = section !== "preferences";
  const showPreferences = section !== "profiles";

  const handleZeroclawDoctorUiToggle = useCallback((nextChecked: boolean) => {
    setShowZeroclawDoctorUi(nextChecked);
    ua.setZeroclawDoctorUiPreference(nextChecked)
      .then((prefs) => {
        setShowZeroclawDoctorUi(Boolean(prefs.showZeroclawDoctorUi));
      })
      .catch((e) => {
        setShowZeroclawDoctorUi((current) => !current);
        const errorText = e instanceof Error ? e.message : String(e);
        toast.error(t("settings.zeroclawDoctorUiSaveFailed", { error: errorText }));
      });
  }, [t, ua]);

  const handleRescueBotUiToggle = useCallback((nextChecked: boolean) => {
    setShowRescueBotUi(nextChecked);
    ua.setRescueBotUiPreference(nextChecked)
      .then((prefs) => {
        setShowRescueBotUi(Boolean(prefs.showRescueBotUi));
      })
      .catch((e) => {
        setShowRescueBotUi((current) => !current);
        const errorText = e instanceof Error ? e.message : String(e);
        toast.error(t("settings.rescueBotUiSaveFailed", { error: errorText }));
      });
  }, [t, ua]);

  const handleSshTransferSpeedUiToggle = useCallback((nextChecked: boolean) => {
    setShowSshTransferSpeedUi(nextChecked);
    ua.setSshTransferSpeedUiPreference(nextChecked)
      .then((prefs) => {
        setShowSshTransferSpeedUi(Boolean(prefs.showSshTransferSpeedUi));
      })
      .catch((e) => {
        setShowSshTransferSpeedUi((current) => !current);
        const errorText = e instanceof Error ? e.message : String(e);
        toast.error(t("settings.sshTransferSpeedUiSaveFailed", { error: errorText }));
      });
  }, [t, ua]);

  useEffect(() => {
    if (!zeroclawPrefsLoadedRef.current) return;
    const next = zeroclawModel.trim();
    if (next === zeroclawLastSavedRef.current) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      ua.setZeroclawModelPreference(next.length > 0 ? next : null)
        .then((prefs) => {
          if (cancelled) return;
          const persisted = prefs.zeroclawModel || "";
          zeroclawLastSavedRef.current = persisted.trim();
          if (persisted !== zeroclawModel) {
            setZeroclawModel(persisted);
          }
        })
        .catch((e) => {
          if (cancelled) return;
          const errorText = e instanceof Error ? e.message : String(e);
          toast.error(t("settings.zeroclawModelSaveFailed", { error: errorText }));
        });
    }, 350);
    return () => { cancelled = true; window.clearTimeout(timer); };
  }, [ua, zeroclawModel, t]);

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t('settings.title')}</h2>

      {/* ---- Model Profiles ---- */}
      {showProfiles && !ua.isRemote && (
        <p className="text-sm text-muted-foreground mb-4">
          {t('settings.oauthHint')}
        </p>
      )}

          <div className="space-y-3">
            {/* Preferences: Version, Language, Theme */}
            {showPreferences && (
            <Card>
              <CardContent className="space-y-4">
                {/* Version */}
                <div className="flex items-center justify-between flex-wrap gap-2">
                  <Label className="text-sm font-semibold">{t('settings.currentVersion')}</Label>
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="text-sm font-medium">{appVersion ? `v${appVersion}` : "..."}</span>
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={handleCheckForUpdates}
                      disabled={appUpdateChecking || appUpdating}
                    >
                      {appUpdateChecking ? t('settings.checkingUpdates') : t('settings.checkForUpdates')}
                    </Button>
                  </div>
                </div>
                {!appUpdateChecking && appUpdate && !appUpdating && (
                  <div className="flex items-center gap-2">
                    <Badge variant="outline" className="text-primary border-primary">
                      {t('settings.updateAvailable', { version: appUpdate.version })}
                    </Badge>
                    <Button size="sm" onClick={handleAppUpdate}>
                      {t('settings.updateRestart')}
                    </Button>
                  </div>
                )}
                {appUpdating && (
                  <div className="flex items-center gap-2">
                    <Badge variant="outline" className="text-muted-foreground">
                      {appUpdateProgress !== null && appUpdateProgress < 100
                        ? t('settings.downloading', { progress: appUpdateProgress })
                        : appUpdateProgress === 100
                          ? t('settings.installing')
                          : t('settings.preparing')}
                    </Badge>
                    {appUpdateProgress !== null && appUpdateProgress < 100 && (
                      <div className="w-32 h-1.5 bg-muted rounded-full overflow-hidden">
                        <div
                          className="h-full bg-primary rounded-full transition-all"
                          style={{ width: `${appUpdateProgress}%` }}
                        />
                      </div>
                    )}
                  </div>
                )}

                <div className="h-px bg-border" />

                {/* Language & Theme */}
                <div className="flex items-center justify-between flex-wrap gap-3">
                  <div className="flex items-center gap-3">
                    <Label className="text-sm font-semibold shrink-0">{t('settings.language')}</Label>
                    <Select
                      value={i18n.language?.startsWith('zh') ? 'zh' : 'en'}
                      onValueChange={(val) => i18n.changeLanguage(val)}
                    >
                      <SelectTrigger className="w-[140px]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="en">English</SelectItem>
                        <SelectItem value="zh">简体中文</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="flex items-center gap-3">
                    <Label className="text-sm font-semibold shrink-0">{t('settings.theme')}</Label>
                    <Select value={theme} onValueChange={setTheme}>
                      <SelectTrigger className="w-[140px]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="light">{t('settings.themeLight')}</SelectItem>
                        <SelectItem value="dark">{t('settings.themeDark')}</SelectItem>
                        <SelectItem value="system">{t('settings.themeSystem')}</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="flex items-center gap-3">
                    <Label className="text-sm font-semibold shrink-0">{t('settings.font')}</Label>
                    <Select value={font} onValueChange={(val) => setFont(val as UiFont)}>
                      <SelectTrigger className="w-[160px]">
                        <SelectValue />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="wenkai">{t('settings.fontWenkai')}</SelectItem>
                        <SelectItem value="nunito">{t('settings.fontNunito')}</SelectItem>
                        <SelectItem value="system">{t('settings.fontSystem')}</SelectItem>
                        <SelectItem value="serif">{t('settings.fontSerif')}</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                </div>

                <div className="h-px bg-border" />

                {showZeroclawDoctorUi && (
                  <div className="flex items-center gap-3 flex-wrap">
                    <Label className="text-sm font-semibold shrink-0">{t("settings.zeroclawModel")}</Label>
                    <div className="w-[320px] max-w-full">
                      <Select
                        value={
                          zeroclawModel && zeroclawModelCandidates.includes(zeroclawModel)
                            ? zeroclawModel
                            : "__none__"
                        }
                        onValueChange={(val) => {
                          const model = val === "__none__" ? "" : val;
                          setZeroclawModel(model);
                          // Optimistically update the displayed runtime target so
                          // "current preferred model" reflects the choice instantly.
                          if (model) {
                            const slashIdx = model.indexOf("/");
                            const provider = slashIdx > 0 ? model.slice(0, slashIdx) : "";
                            const modelName = slashIdx > 0 ? model.slice(slashIdx + 1) : model;
                            setZeroclawTarget((prev) => ({
                              ...prev,
                              provider,
                              model: modelName,
                              source: "preferred",
                              preferredModel: model,
                              providerOrder: prev?.providerOrder ?? [],
                            }));
                          }
                        }}

                      >
                        <SelectTrigger>
                          <SelectValue placeholder={t("settings.zeroclawModelPlaceholder")} />
                        </SelectTrigger>
                        <SelectContent>
                          <SelectItem value="__none__">
                            <span className="text-muted-foreground">{t("home.notSet")}</span>
                          </SelectItem>
                          {zeroclawModelCandidates.map((model) => (
                            <SelectItem key={model} value={model}>
                              {model}
                            </SelectItem>
                          ))}
                        </SelectContent>
                      </Select>
                    </div>
                    {zeroclawModelCandidates.length === 0 && (
                      <p className="text-xs text-muted-foreground basis-full mt-1">
                        {onNavigateToProfiles ? (
                          <button
                            type="button"
                            className="underline hover:text-foreground transition-colors"
                            onClick={onNavigateToProfiles}
                          >
                            {t("settings.zeroclawNoProfilesLink")}
                          </button>
                        ) : (
                          t("settings.zeroclawNoProfiles")
                        )}
                      </p>
                    )}
                    <div className="ml-auto text-right text-xs text-muted-foreground min-w-[240px]">
                      {zeroclawUsageLoading ? (
                        <div>{t("settings.zeroclawUsageLoading")}</div>
                      ) : (
                        <>
                          <div>
                            {t("settings.zeroclawUsageTotalTokens", {
                              count: zeroclawUsage?.totalTokens || 0,
                            })}
                          </div>
                          <div>
                            {t("settings.zeroclawUsageCalls", {
                              count: zeroclawUsage?.totalCalls || 0,
                            })}
                          </div>
                        </>
                      )}
                      <div>
                        {t("settings.zeroclawEffectiveModelLabel", {
                          model: zeroclawTargetText,
                        })}
                      </div>
                    </div>
                  </div>
                )}
              </CardContent>
            </Card>
            )}

            {/* Profiles list */}
            {showProfiles && (
            <Card>
              <CardHeader>
                <div className="flex items-center justify-between">
                  <CardTitle>{t('settings.modelProfiles')}</CardTitle>
                  <Button size="sm" onClick={openAddProfile}>{t('settings.addProfile')}</Button>
                </div>
              </CardHeader>
              <CardContent>
                {profiles === null ? (
                  <p className="text-muted-foreground">{t('settings.loadingProfiles')}</p>
                ) : profiles.length === 0 ? (
                  <p className="text-muted-foreground">{t('settings.noProfiles')}</p>
                ) : null}
                <div className="grid gap-2">
                  {(profiles || []).map((profile) => {
                    const credential = resolveProfileCredentialView(
                      profile,
                      resolvedCredentialMap.get(profile.id),
                    );
                    const statusLower = credential.status.trim().toLowerCase();
                    const credentialStatusText =
                      credential.kind === "oauth" && statusLower !== "..."
                        ? (credential.resolved
                          ? t("settings.credentialStatusOauthReady")
                          : credential.status)
                        : credential.status;
                    const showCredentialRef = credential.kind === "env_ref";
                    const showCredentialStatus = credential.kind !== "env_ref";
                    return (
                      <div
                        key={profile.id}
                        className="border border-border p-2.5 rounded-lg"
                      >
                        <div className="flex justify-between items-center">
                          <strong>{profile.provider}/{profile.model}</strong>
                          {profile.enabled ? (
                            <Badge className="bg-blue-500/10 text-blue-600 dark:bg-blue-500/15 dark:text-blue-400">
                              {t('settings.enabled')}
                            </Badge>
                          ) : (
                            <Badge className="bg-red-500/10 text-red-600 dark:bg-red-500/15 dark:text-red-400">
                              {t('settings.disabled')}
                            </Badge>
                          )}
                        </div>
                        <div className="text-sm text-muted-foreground mt-1">
                          {t('settings.credential')}: {t(`settings.credentialKind.${credential.kind}`)}
                        </div>
                        {showCredentialRef && (
                          <div className="text-sm text-muted-foreground mt-0.5">
                            {t("settings.credentialRef")}: {credential.authRef || "-"}
                          </div>
                        )}
                        {showCredentialStatus && (
                          <div className="text-sm text-muted-foreground mt-0.5">
                            {t("settings.credentialStatus")}: {credentialStatusText}
                          </div>
                        )}
                        {profile.baseUrl && (
                          <div className="text-sm text-muted-foreground mt-0.5">
                            URL: {profile.baseUrl}
                          </div>
                        )}
                        <div className="flex gap-1.5 mt-1.5">
                          {ENABLE_PROFILE_TEST_BUTTON && (
                            <Button
                              size="sm"
                              variant="outline"
                              type="button"
                              onClick={() => testProfile(profile)}
                              disabled={testingProfileId === profile.id}
                            >
                              {testingProfileId === profile.id ? t('settings.testing') : t('settings.test')}
                            </Button>
                          )}
                          <Button
                            size="sm"
                            variant="outline"
                            type="button"
                            onClick={() => toggleProfileEnabled(profile)}
                          >
                            {profile.enabled ? t('settings.disable') : t('settings.enable')}
                          </Button>
                          <Button
                            size="sm"
                            variant="outline"
                            type="button"
                            onClick={() => editProfile(profile)}
                          >
                            {t('settings.edit')}
                          </Button>
                          <AlertDialog>
                            <AlertDialogTrigger asChild>
                              <Button size="sm" variant="destructive" type="button">
                                {t('settings.delete')}
                              </Button>
                            </AlertDialogTrigger>
                            <AlertDialogContent>
                              <AlertDialogHeader>
                                <AlertDialogTitle>{t('settings.deleteProfileTitle')}</AlertDialogTitle>
                                <AlertDialogDescription>
                                  {t('settings.deleteProfileDescription', { name: `${profile.provider}/${profile.model}` })}
                                </AlertDialogDescription>
                              </AlertDialogHeader>
                              <AlertDialogFooter>
                                <AlertDialogCancel>{t('settings.cancel')}</AlertDialogCancel>
                                <AlertDialogAction
                                  className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                                  onClick={() => deleteProfile(profile.id)}
                                >
                                  {t('settings.delete')}
                                </AlertDialogAction>
                              </AlertDialogFooter>
                            </AlertDialogContent>
                          </AlertDialog>
                        </div>
                      </div>
                    );
                  })}
                </div>
              </CardContent>
            </Card>
            )}

            {showPreferences && (
              <Card>
                <CardContent>
                  <details className="rounded-lg border border-border/60 bg-muted/20 px-3 py-2">
                    <summary className="cursor-pointer text-sm font-semibold text-foreground">
                      {t("settings.alphaFeatures")}
                    </summary>
                    <div className="mt-3 space-y-3">
                      <p className="text-xs text-muted-foreground">
                        {t("settings.alphaFeaturesDescription")}
                      </p>
                      <div className="flex items-center justify-between gap-2 flex-wrap">
                        <Label className="text-sm font-medium">{t("settings.alphaEnableZeroclawUi")}</Label>
                        <Checkbox
                          checked={showZeroclawDoctorUi}
                          onCheckedChange={(checked) => handleZeroclawDoctorUiToggle(checked === true)}
                          aria-label={t("settings.alphaEnableZeroclawUi")}
                          className="h-5 w-5"
                        />
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {t("settings.alphaEnableZeroclawUiHint")}
                      </p>
                      <div className="flex items-center justify-between gap-2 flex-wrap">
                        <Label className="text-sm font-medium">{t("settings.alphaEnableRescueBotUi")}</Label>
                        <Checkbox
                          checked={showRescueBotUi}
                          onCheckedChange={(checked) => handleRescueBotUiToggle(checked === true)}
                          aria-label={t("settings.alphaEnableRescueBotUi")}
                          className="h-5 w-5"
                        />
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {t("settings.alphaEnableRescueBotUiHint")}
                      </p>
                      <div className="flex items-center justify-between gap-2 flex-wrap">
                        <Label className="text-sm font-medium">{t("settings.alphaEnableSshTransferSpeedUi")}</Label>
                        <Checkbox
                          checked={showSshTransferSpeedUi}
                          onCheckedChange={(checked) => handleSshTransferSpeedUiToggle(checked === true)}
                          aria-label={t("settings.alphaEnableSshTransferSpeedUi")}
                          className="h-5 w-5"
                        />
                      </div>
                      <p className="text-xs text-muted-foreground">
                        {t("settings.alphaEnableSshTransferSpeedUiHint")}
                      </p>
                    </div>
                  </details>
                </CardContent>
              </Card>
            )}
          </div>

      {message && (
        <p className="text-sm text-muted-foreground mt-3">{message}</p>
      )}

      {/* Add / Edit Profile Dialog */}
      <Dialog open={profileDialogOpen} onOpenChange={(open) => {
        setProfileDialogOpen(open);
        if (!open) {
          setCredentialSource("manual");
          setForm(emptyForm());
          setOauthAuthorizeUrl("");
          setOauthRedirectInput("");
          setOauthStarting(false);
          setOauthCompleting(false);
        }
      }}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{form.id ? t('settings.editProfile') : t('settings.addProfile')}</DialogTitle>
          </DialogHeader>
          <form onSubmit={upsert} className="space-y-4">
            <div className="space-y-1.5">
              <Label>{t('settings.provider')}</Label>
              <AutocompleteField
                value={form.provider}
                onChange={(val) => {
                  const nextSource: CredentialSource = providerUsesOAuthAuth(val)
                    ? (credentialSource === "manual" ? "manual" : "oauth")
                    : (credentialSource === "oauth" ? "env" : credentialSource);
                  setCredentialSource(nextSource);
                  setForm((p) => ({
                    ...p,
                    provider: val,
                    model: "",
                    authRef: p.id
                      ? p.authRef
                      : providerUsesOAuthAuth(val)
                        ? defaultOauthAuthRef(val)
                        : (nextSource === "env" ? (p.authRef || defaultEnvAuthRef(val)) : p.authRef),
                  }));
                }}
                onFocus={ensureCatalog}
                options={providerCandidates.map((provider) => ({
                  value: provider,
                  label: provider,
                }))}
                placeholder="e.g. openai"
              />
            </div>

            <div className="space-y-1.5">
              <Label>{t('settings.model')}</Label>
              <AutocompleteField
                value={form.model}
                onChange={(val) =>
                  setForm((p) => ({ ...p, model: val }))
                }
                onFocus={ensureCatalog}
                options={modelCandidates.map((m) => ({
                  value: m.id,
                  label: m.name || m.id,
                }))}
                placeholder="e.g. gpt-4o"
              />
            </div>

            <div className="space-y-1.5">
              <Label>{t('settings.credentialSource')}</Label>
              <Select
                value={credentialSource}
                onValueChange={(val) => {
                  const next = val as CredentialSource;
                  if (next === "oauth" && !providerUsesOAuthAuth(form.provider)) {
                    return;
                  }
                  setCredentialSource(next);
                  setForm((p) => {
                    if (next === "oauth") {
                      const oauthRef = p.authRef.trim();
                      return {
                        ...p,
                        apiKey: "",
                        authRef: oauthRef && !isEnvVarLikeAuthRef(oauthRef)
                          ? oauthRef
                          : defaultOauthAuthRef(p.provider),
                      };
                    }
                    if (next === "env") {
                      const currentRef = p.authRef.trim();
                      return {
                        ...p,
                        authRef: currentRef || defaultEnvAuthRef(p.provider),
                      };
                    }
                    return p;
                  });
                }}
              >
                <SelectTrigger>
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {providerUsesOAuthAuth(form.provider) && (
                    <SelectItem value="oauth">{t("settings.credentialSourceOauth")}</SelectItem>
                  )}
                  <SelectItem value="env">{t("settings.credentialSourceEnv")}</SelectItem>
                  <SelectItem value="manual">{t("settings.credentialSourceManual")}</SelectItem>
                </SelectContent>
              </Select>
            </div>

            {credentialSource === "oauth" && providerUsesOAuthAuth(form.provider) && (
              <div className="rounded-md border border-border/70 bg-muted/30 px-3 py-2 text-xs text-muted-foreground space-y-2">
                <p>{t("settings.oauthProviderHint", { provider: normalizeOauthProvider(form.provider) })}</p>
                <div className="flex flex-wrap gap-2">
                  <Button
                    type="button"
                    size="sm"
                    variant="outline"
                    onClick={startOauthLogin}
                    disabled={oauthStarting || oauthCompleting}
                  >
                    {oauthStarting ? t("settings.oauthStarting") : t("settings.oauthStart")}
                  </Button>
                  {oauthAuthorizeUrl && (
                    <Button
                      type="button"
                      size="sm"
                      variant="outline"
                      onClick={() => {
                        ua.openUrl(oauthAuthorizeUrl).catch((e) => {
                          setMessage(t("settings.oauthOpenLinkFailed", { error: String(e) }));
                        });
                      }}
                    >
                      {t("settings.oauthOpenLink")}
                    </Button>
                  )}
                </div>
                {oauthAuthorizeUrl && (
                  <p className="font-mono break-all">{oauthAuthorizeUrl}</p>
                )}
                <div className="space-y-1">
                  <Label>{t("settings.oauthRedirectInputLabel")}</Label>
                  <Input
                    placeholder={t("settings.oauthRedirectInputPlaceholder")}
                    value={oauthRedirectInput}
                    onChange={(e) => setOauthRedirectInput(e.target.value)}
                  />
                </div>
                <Button
                  type="button"
                  size="sm"
                  onClick={completeOauthAndSave}
                  disabled={oauthStarting || oauthCompleting || !oauthRedirectInput.trim()}
                >
                  {oauthCompleting ? t("settings.oauthCompleting") : t("settings.oauthCompleteAndSave")}
                </Button>
              </div>
            )}

            {credentialSource === "env" && (
              <div className="space-y-1.5">
                <Label>{t('settings.authRef')}</Label>
                <Input
                  placeholder={defaultEnvAuthRef(form.provider) || "OPENAI_API_KEY"}
                  value={form.authRef}
                  onChange={(e) =>
                    setForm((p) => ({ ...p, authRef: e.target.value }))
                  }
                />
                <p className="text-xs text-muted-foreground">
                  {t("settings.credentialSourceEnvHint")}
                </p>
              </div>
            )}

            {credentialSource === "manual" && (
              <div className="space-y-1.5">
                <Label>{t('settings.apiKey')}</Label>
                <Input
                  type="password"
                  placeholder={form.id
                    ? t('settings.apiKeyUnchanged')
                    : (authSuggestion?.hasKey || form.useCustomUrl || providerSupportsOptionalApiKey(form.provider))
                      ? t('settings.apiKeyOptional')
                      : t('settings.apiKeyPlaceholder')}
                  value={form.apiKey}
                  onChange={(e) =>
                    setForm((p) => ({ ...p, apiKey: e.target.value }))
                  }
                />
                {!form.id && authSuggestion?.hasKey && (
                  <p className="text-xs text-muted-foreground">
                    {t('settings.keyAvailable', { source: authSuggestion.source })}
                  </p>
                )}
              </div>
            )}

            <div className="flex items-center gap-2">
              <Checkbox
                id="custom-url"
                checked={form.useCustomUrl}
                onCheckedChange={(checked) =>
                  setForm((p) => ({ ...p, useCustomUrl: checked === true }))
                }
              />
              <Label htmlFor="custom-url">{t('settings.customBaseUrl')}</Label>
            </div>

            {form.useCustomUrl && (
              <div className="space-y-1.5">
                <Label>{t('settings.baseUrl')}</Label>
                <Input
                  placeholder="e.g. https://api.openai.com/v1"
                  value={form.baseUrl}
                  onChange={(e) =>
                    setForm((p) => ({ ...p, baseUrl: e.target.value }))
                  }
                />
              </div>
            )}

            <div className="flex items-center gap-2">
              <Checkbox
                id="profile-enabled"
                checked={form.enabled}
                onCheckedChange={(checked) =>
                  setForm((p) => ({ ...p, enabled: checked === true }))
                }
              />
              <Label htmlFor="profile-enabled">{t('settings.profileEnabled')}</Label>
            </div>

            <DialogFooter>
              {form.id && (
                <AlertDialog>
                  <AlertDialogTrigger asChild>
                    <Button type="button" variant="destructive" className="mr-auto">
                      {t('settings.delete')}
                    </Button>
                  </AlertDialogTrigger>
                  <AlertDialogContent>
                    <AlertDialogHeader>
                      <AlertDialogTitle>{t('settings.deleteProfileTitle')}</AlertDialogTitle>
                      <AlertDialogDescription>
                        {t('settings.deleteProfileDescription', { name: `${form.provider}/${form.model}` })}
                      </AlertDialogDescription>
                    </AlertDialogHeader>
                    <AlertDialogFooter>
                      <AlertDialogCancel>{t('settings.cancel')}</AlertDialogCancel>
                      <AlertDialogAction
                        className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
                        onClick={() => { deleteProfile(form.id); setProfileDialogOpen(false); }}
                      >
                        {t('settings.delete')}
                      </AlertDialogAction>
                    </AlertDialogFooter>
                  </AlertDialogContent>
                </AlertDialog>
              )}
              <Button type="button" variant="outline" onClick={() => setProfileDialogOpen(false)}>
                {t('settings.cancel')}
              </Button>
              <Button type="submit">{t('settings.save')}</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>

    </section>
  );
}
