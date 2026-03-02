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
  apiKey: string;
  useCustomUrl: boolean;
  baseUrl: string;
  enabled: boolean;
};

const MODEL_CATALOG_CACHE_TTL_MS = 5 * 60_000;
const ENABLE_PROFILE_TEST_BUTTON = true;
const ZEROCLAW_SUPPORTED_PROVIDERS = new Set([
  "openrouter",
  "openai",
  "openai-codex",
  "anthropic",
  "github-copilot",
  "copilot",
  "gemini",
  "google",
  "google-vertex",
  "google-gemini-cli",
  "google-antigravity",
  "moonshot",
  "kimi-code",
  "kimi-coding",
]);
let modelCatalogCache: { value: ModelCatalogProvider[]; expiresAt: number } | null = null;
let profilesExtractedOnce = false;

function emptyForm(): ProfileForm {
  return {
    id: "",
    provider: "",
    model: "",
    apiKey: "",
    useCustomUrl: false,
    baseUrl: "",
    enabled: true,
  };
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
  const [profileDialogOpen, setProfileDialogOpen] = useState(false);
  const [message, setMessage] = useState("");
  const [authSuggestion, setAuthSuggestion] = useState<ProviderAuthSuggestion | null>(null);
  const [testingProfileId, setTestingProfileId] = useState<string | null>(null);
  const [zeroclawModel, setZeroclawModel] = useState("");
  const [zeroclawSaving, setZeroclawSaving] = useState(false);
  const [zeroclawUsage, setZeroclawUsage] = useState<ZeroclawUsageStats | null>(null);
  const [zeroclawUsageLoading, setZeroclawUsageLoading] = useState(true);
  const [zeroclawTarget, setZeroclawTarget] = useState<ZeroclawRuntimeTarget | null>(null);
  const [zeroclawTargetLoading, setZeroclawTargetLoading] = useState(true);
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
      })
      .catch((e) => console.error("Failed to load app preferences:", e));
  }, [ua]);

  useEffect(() => {
    let cancelled = false;
    const loadStats = () => {
      setZeroclawUsageLoading(true);
      setZeroclawTargetLoading(true);
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

  const maskedKeyMap = useMemo(() => {
    const map = new Map<string, string>();
    for (const entry of apiKeys) {
      map.set(entry.profileId, entry.maskedKey);
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
        (p) => p.provider === form.provider && maskedKeyMap.has(p.id) && maskedKeyMap.get(p.id) !== "..."
      );
      if (existing) {
        setAuthSuggestion({
          hasKey: true,
          source: `existing profile (${existing.provider}/${existing.model})`,
          authRef: existing.authRef || "",
        });
      } else {
        setAuthSuggestion(null);
      }
    } else {
      ua.resolveProviderAuth(form.provider)
        .then(setAuthSuggestion)
        .catch((e) => { console.error("Failed to resolve provider auth:", e); setAuthSuggestion(null); });
    }
  }, [form.provider, form.id, ua, profiles, maskedKeyMap]);

  const modelCandidates = useMemo(() => {
    const found = catalog.find((c) => c.provider === form.provider);
    return found?.models || [];
  }, [catalog, form.provider]);

  const zeroclawModelCandidates = useMemo(() => {
    const fromProfiles = (profiles || [])
      .filter((profile) => profile.enabled)
      .filter((profile) =>
        ZEROCLAW_SUPPORTED_PROVIDERS.has(profile.provider.trim().toLowerCase()),
      )
      .map((profile) => profileToModelValue(profile));
    return Array.from(new Set(fromProfiles)).sort((a, b) => a.localeCompare(b));
  }, [profiles]);

  const upsert = (event: FormEvent) => {
    event.preventDefault();
    if (!form.provider || !form.model) {
      setMessage(t('settings.providerModelRequired'));
      return;
    }
    if (!ua.isRemote && !form.apiKey && !form.id && !authSuggestion?.hasKey) {
      setMessage(t('settings.apiKeyRequired'));
      return;
    }
    const profileData: ModelProfile = {
      id: form.id || "",
      name: `${form.provider}/${form.model}`,
      provider: form.provider,
      model: form.model,
      authRef: (!form.apiKey && authSuggestion?.authRef) ? authSuggestion.authRef : "",
      apiKey: form.apiKey || undefined,
      baseUrl: form.useCustomUrl && form.baseUrl ? form.baseUrl : undefined,
      enabled: form.enabled,
    };
    ua.upsertModelProfile(profileData)
      .then(() => {
        setMessage(t('settings.profileSaved'));
        setForm(emptyForm());
        setProfileDialogOpen(false);
        refreshProfiles();
        onDataChange?.();
      })
      .catch((e) => setMessage(t('settings.saveFailed', { error: String(e) })));
  };

  const editProfile = (profile: ModelProfile) => {
    setForm({
      id: profile.id,
      provider: profile.provider,
      model: profile.model,
      apiKey: "",
      useCustomUrl: !!profile.baseUrl,
      baseUrl: profile.baseUrl || "",
      enabled: profile.enabled,
    });
    setProfileDialogOpen(true);
  };

  const openAddProfile = () => {
    setForm(emptyForm());
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

  useEffect(() => {
    if (!zeroclawPrefsLoadedRef.current) return;
    const next = zeroclawModel.trim();
    if (next === zeroclawLastSavedRef.current) return;
    const timer = window.setTimeout(() => {
      setZeroclawSaving(true);
      ua.setZeroclawModelPreference(next.length > 0 ? next : null)
        .then((prefs) => {
          const persisted = prefs.zeroclawModel || "";
          zeroclawLastSavedRef.current = persisted.trim();
          if (persisted !== zeroclawModel) {
            setZeroclawModel(persisted);
          }
        })
        .catch((e) => {
          const errorText = e instanceof Error ? e.message : String(e);
          toast.error(t("settings.zeroclawModelSaveFailed", { error: errorText }));
        })
        .finally(() => setZeroclawSaving(false));
    }, 350);
    return () => window.clearTimeout(timer);
  }, [ua, zeroclawModel, t]);

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t('settings.title')}</h2>

      {/* ---- Model Profiles ---- */}
      {showProfiles && !ua.isRemote && (
        <p className="text-sm text-muted-foreground mb-4">
          {t('settings.oauthHint')}
          <code className="mx-1 px-1.5 py-0.5 bg-muted rounded text-xs">openclaw models auth login</code>
          {t('settings.or')}
          <code className="mx-1 px-1.5 py-0.5 bg-muted rounded text-xs">openclaw models auth login-github-copilot</code>.
          {t('settings.oauthHintSuffix')}
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

                <div className="flex items-center gap-3 flex-wrap">
                  <Label className="text-sm font-semibold shrink-0">{t("settings.zeroclawModel")}</Label>
                  <div className="w-[320px] max-w-full">
                    <Select
                      value={zeroclawModel ? (zeroclawModelCandidates.includes(zeroclawModel) ? zeroclawModel : "__raw__") : "__none__"}
                      onValueChange={(val) => {
                        if (val === "__raw__") return;
                        setZeroclawModel(val === "__none__" ? "" : val);
                      }}
                      disabled={zeroclawSaving}
                    >
                      <SelectTrigger>
                        <SelectValue placeholder={t("settings.zeroclawModelPlaceholder")} />
                      </SelectTrigger>
                      <SelectContent>
                        <SelectItem value="__none__">
                          <span className="text-muted-foreground">{t("home.notSet")}</span>
                        </SelectItem>
                        {zeroclawModel && !zeroclawModelCandidates.includes(zeroclawModel) && (
                          <SelectItem value="__raw__">{zeroclawModel}</SelectItem>
                        )}
                        {zeroclawModelCandidates.map((model) => (
                          <SelectItem key={model} value={model}>
                            {model}
                          </SelectItem>
                        ))}
                      </SelectContent>
                    </Select>
                  </div>
                  {zeroclawSaving && (
                    <span className="text-xs text-muted-foreground">{t("settings.saving")}</span>
                  )}
                  {zeroclawModelCandidates.length === 0 && !zeroclawSaving && (
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
                  {(profiles || []).map((profile) => (
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
                        {t('settings.apiKey')}: {maskedKeyMap.get(profile.id) || "..."}
                      </div>
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
                  ))}
                </div>
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
        if (!open) setForm(emptyForm());
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
                onChange={(val) =>
                  setForm((p) => ({ ...p, provider: val, model: "" }))
                }
                onFocus={ensureCatalog}
                options={catalog.map((c) => ({
                  value: c.provider,
                  label: c.provider,
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
              <Label>{t('settings.apiKey')}</Label>
              <Input
                type="password"
                placeholder={form.id ? t('settings.apiKeyUnchanged') : authSuggestion?.hasKey ? t('settings.apiKeyOptional') : t('settings.apiKeyPlaceholder')}
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
