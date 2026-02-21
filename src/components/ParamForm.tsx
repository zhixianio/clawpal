import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import i18n from "@/i18n";
import type { AgentOverview, ModelProfile, Recipe, RecipeParam } from "../lib/types";
import { useApi } from "@/lib/use-api";
import { useInstance } from "@/lib/instance-context";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

function validateField(param: RecipeParam, value: string): string | null {
  const trim = value.trim();
  if (param.required && trim.length === 0) {
    return i18n.t('paramForm.isRequired', { label: param.label });
  }
  // Select-based types only need required check
  if (param.type === "discord_guild" || param.type === "discord_channel" || param.type === "model_profile" || param.type === "agent") {
    return null;
  }
  if (param.minLength != null && trim.length < param.minLength) {
    return i18n.t('paramForm.tooShort', { label: param.label });
  }
  if (param.maxLength != null && trim.length > param.maxLength) {
    return i18n.t('paramForm.tooLong', { label: param.label });
  }
  if (param.pattern && trim.length > 0) {
    try {
      if (!new RegExp(param.pattern).test(trim)) {
        return i18n.t('paramForm.invalidFormat', { label: param.label });
      }
    } catch {
      return i18n.t('paramForm.invalidRule', { label: param.label });
    }
  }
  return null;
}

export function ParamForm({
  recipe,
  values,
  onChange,
  onSubmit,
  submitLabel = "Preview",
}: {
  recipe: Recipe;
  values: Record<string, string>;
  onChange: (id: string, value: string) => void;
  onSubmit: () => void;
  submitLabel?: string;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const { discordGuildChannels } = useInstance();
  const [touched, setTouched] = useState<Record<string, boolean>>({});
  const [modelProfiles, setModelProfiles] = useState<ModelProfile[]>([]);
  const [agents, setAgents] = useState<AgentOverview[]>([]);

  // Lazily load model profiles if any param needs them
  const needsProfiles = recipe.params.some((p) => p.type === "model_profile");
  useEffect(() => {
    if (!needsProfiles) return;
    ua.listModelProfiles().then(setModelProfiles).catch((e) => console.error("Failed to load model profiles:", e));
  }, [needsProfiles, ua]);

  // Lazily load agents if any param needs them
  const needsAgents = recipe.params.some((p) => p.type === "agent");
  useEffect(() => {
    if (!needsAgents) return;
    ua.listAgents().then(setAgents).catch((e) => console.error("Failed to load agents:", e));
  }, [needsAgents, ua]);

  const uniqueGuilds = useMemo(() => {
    const seen = new Map<string, string>();
    for (const gc of discordGuildChannels) {
      if (!seen.has(gc.guildId)) {
        seen.set(gc.guildId, gc.guildName);
      }
    }
    return Array.from(seen, ([id, name]) => ({ id, name }));
  }, [discordGuildChannels]);

  const filteredChannels = useMemo(() => {
    const guildId = values["guild_id"];
    if (!guildId) return [];
    return discordGuildChannels.filter((gc) => gc.guildId === guildId);
  }, [discordGuildChannels, values]);

  const isParamVisible = (param: RecipeParam) => {
    if (!param.dependsOn) return true;
    return values[param.dependsOn] === "true";
  };

  const errors = useMemo(() => {
    const next: Record<string, string> = {};
    for (const param of recipe.params) {
      if (!isParamVisible(param)) continue;
      const err = validateField(param, values[param.id] || "");
      if (err) {
        next[param.id] = err;
      }
    }
    return next;
  }, [recipe.params, values]);
  const hasError = Object.keys(errors).length > 0;

  function renderParam(param: RecipeParam) {
    if (param.type === "boolean") {
      return (
        <div className="flex items-center gap-2">
          <Checkbox
            id={param.id}
            checked={values[param.id] === "true"}
            onCheckedChange={(checked) => {
              onChange(param.id, checked === true ? "true" : "false");
            }}
          />
          <Label htmlFor={param.id} className="font-normal">{param.label}</Label>
        </div>
      );
    }

    if (param.type === "discord_guild") {
      return (
        <Select
          value={values[param.id] || undefined}
          onValueChange={(val) => {
            onChange(param.id, val);
            setTouched((prev) => ({ ...prev, [param.id]: true }));
            // Clear channel selection when guild changes
            const channelParam = recipe.params.find((p) => p.type === "discord_channel");
            if (channelParam && values[channelParam.id]) {
              onChange(channelParam.id, "");
            }
          }}
        >
          <SelectTrigger id={param.id} size="sm" className="w-full">
            <SelectValue placeholder={t('paramForm.selectGuild')} />
          </SelectTrigger>
          <SelectContent>
            {uniqueGuilds.map((g) => (
              <SelectItem key={g.id} value={g.id}>
                {g.name}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }

    if (param.type === "discord_channel") {
      const guildSelected = !!values["guild_id"];
      return (
        <Select
          value={values[param.id] || undefined}
          onValueChange={(val) => {
            onChange(param.id, val);
            setTouched((prev) => ({ ...prev, [param.id]: true }));
          }}
          disabled={!guildSelected}
        >
          <SelectTrigger id={param.id} size="sm" className="w-full">
            <SelectValue
              placeholder={guildSelected ? t('paramForm.selectChannel') : t('paramForm.selectGuildFirst')}
            />
          </SelectTrigger>
          <SelectContent>
            {filteredChannels.map((c) => (
              <SelectItem key={c.channelId} value={c.channelId}>
                {c.channelName}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }

    if (param.type === "agent") {
      return (
        <Select
          value={values[param.id] || undefined}
          onValueChange={(val) => {
            onChange(param.id, val);
            setTouched((prev) => ({ ...prev, [param.id]: true }));
          }}
        >
          <SelectTrigger id={param.id} size="sm" className="w-full">
            <SelectValue placeholder={t('paramForm.selectAgent')} />
          </SelectTrigger>
          <SelectContent>
            {agents.map((a) => (
              <SelectItem key={a.id} value={a.id}>
                {a.emoji ? `${a.emoji} ` : ""}{a.name || a.id}
                <span className="text-muted-foreground ml-1.5 text-xs">({a.id})</span>
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }

    if (param.type === "model_profile") {
      const enabledProfiles = modelProfiles.filter((p) => p.enabled);
      return (
        <Select
          value={values[param.id] || undefined}
          onValueChange={(val) => {
            onChange(param.id, val);
            setTouched((prev) => ({ ...prev, [param.id]: true }));
          }}
        >
          <SelectTrigger id={param.id} size="sm" className="w-full">
            <SelectValue placeholder={t('paramForm.selectModel')} />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="__default__">
              <span className="text-muted-foreground">{t('paramForm.useGlobalDefault')}</span>
            </SelectItem>
            {enabledProfiles.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.provider}/{p.model}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      );
    }

    if (param.type === "textarea") {
      return (
        <Textarea
          id={param.id}
          value={values[param.id] || ""}
          placeholder={param.placeholder}
          onBlur={() => setTouched((prev) => ({ ...prev, [param.id]: true }))}
          onChange={(e) => {
            onChange(param.id, e.target.value);
            setTouched((prev) => ({ ...prev, [param.id]: true }));
          }}
        />
      );
    }

    return (
      <Input
        id={param.id}
        value={values[param.id] || ""}
        placeholder={param.placeholder}
        required={param.required}
        onBlur={() => setTouched((prev) => ({ ...prev, [param.id]: true }))}
        onChange={(e) => {
          onChange(param.id, e.target.value);
          setTouched((prev) => ({ ...prev, [param.id]: true }));
        }}
      />
    );
  }

  return (
    <form className="space-y-4" onSubmit={(e) => {
      e.preventDefault();
      if (hasError) {
        return;
      }
      onSubmit();
    }}>
      {recipe.params.map((param: RecipeParam) => {
        if (!isParamVisible(param)) return null;
        const isBool = param.type === "boolean";
        return (
          <div key={param.id} className="space-y-1.5">
            {!isBool && <Label htmlFor={param.id}>{param.label}</Label>}
            {renderParam(param)}
            {touched[param.id] && errors[param.id] ? (
              <p className="text-sm text-destructive">{errors[param.id]}</p>
            ) : null}
          </div>
        );
      })}
      <Button
        type="submit"
        disabled={hasError}
      >
        {submitLabel}
      </Button>
    </form>
  );
}
