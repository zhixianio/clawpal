import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useApi } from "@/lib/use-api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Checkbox } from "@/components/ui/checkbox";
import { Textarea } from "@/components/ui/textarea";
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
import type { ModelProfile } from "../lib/types";

export interface CreateAgentResult {
  agentId: string;
  persona?: string;
}

export function CreateAgentDialog({
  open,
  onOpenChange,
  modelProfiles,
  onCreated,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  modelProfiles: ModelProfile[];
  onCreated: (result: CreateAgentResult) => void;
}) {
  const { t } = useTranslation();
  const ua = useApi();
  const [agentId, setAgentId] = useState("");
  const [model, setModel] = useState("");
  const [independent, setIndependent] = useState(false);
  const [displayName, setDisplayName] = useState("");
  const [emoji, setEmoji] = useState("");
  const [persona, setPersona] = useState("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState("");

  const reset = () => {
    setAgentId("");
    setModel("");
    setIndependent(false);
    setDisplayName("");
    setEmoji("");
    setPersona("");
    setError("");
  };

  const handleCreate = async () => {
    const id = agentId.trim();
    if (!id) {
      setError(t('createAgent.agentIdRequired'));
      return;
    }
    setCreating(true);
    setError("");
    try {
      // Resolve profile ID to "provider/model" value
      const resolveModelValue = (profileId: string | undefined): string | undefined => {
        if (!profileId || profileId === "__default__") return undefined;
        const profile = modelProfiles.find((p) => p.id === profileId);
        if (!profile) return profileId;
        return profile.model.includes("/")
          ? profile.model
          : `${profile.provider}/${profile.model}`;
      };
      const modelValue = resolveModelValue(model || undefined);

      // Build CLI command for queue
      const command: string[] = ["openclaw", "agents", "add", id, "--non-interactive"];
      if (modelValue) {
        command.push("--model", modelValue);
      }
      if (independent) {
        // --workspace flag triggers independent agent creation
        command.push("--workspace", id);
      }
      await ua.queueCommand(`Create agent: ${id}`, command);

      onOpenChange(false);
      const result: CreateAgentResult = { agentId: id };
      if (persona.trim()) result.persona = persona.trim();
      reset();
      onCreated(result);
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(v) => { if (!v) reset(); onOpenChange(v); }}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t('createAgent.title')}</DialogTitle>
        </DialogHeader>
        <div className="space-y-4 py-2">
          <div className="space-y-1.5">
            <Label>{t('createAgent.agentId')}</Label>
            <Input
              placeholder={t('createAgent.agentIdPlaceholder')}
              value={agentId}
              onChange={(e) => setAgentId(e.target.value)}
            />
            <p className="text-xs text-muted-foreground">
              {t('createAgent.agentIdHint')}
            </p>
          </div>
          <div className="space-y-1.5">
            <Label>{t('createAgent.model')}</Label>
            <Select
              value={model || "__default__"}
              onValueChange={(val) => setModel(val === "__default__" ? "" : val)}
            >
              <SelectTrigger size="sm" className="text-sm">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="__default__">
                  <span className="text-muted-foreground">{t('createAgent.useGlobalDefault')}</span>
                </SelectItem>
                {modelProfiles.map((p) => (
                  <SelectItem key={p.id} value={p.id}>
                    {p.provider}/{p.model}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="flex items-center gap-2">
            <Checkbox
              id="create-agent-independent"
              checked={independent}
              onCheckedChange={(checked) => {
                const val = checked === true;
                setIndependent(val);
                if (!val) {
                  setDisplayName("");
                  setEmoji("");
                  setPersona("");
                }
              }}
            />
            <Label htmlFor="create-agent-independent">{t('createAgent.independent')}</Label>
          </div>
          {independent && (
            <>
              <div className="space-y-1.5">
                <Label>{t('createAgent.displayName')}</Label>
                <Input
                  placeholder={t('createAgent.displayNamePlaceholder')}
                  value={displayName}
                  onChange={(e) => setDisplayName(e.target.value)}
                />
              </div>
              <div className="space-y-1.5">
                <Label>{t('createAgent.emoji')}</Label>
                <Input
                  placeholder="e.g. \uD83E\uDD16"
                  value={emoji}
                  onChange={(e) => setEmoji(e.target.value)}
                  className="w-20"
                />
              </div>
              <div className="space-y-1.5">
                <Label>{t('createAgent.persona')}</Label>
                <Textarea
                  placeholder={t('createAgent.personaPlaceholder')}
                  value={persona}
                  onChange={(e) => setPersona(e.target.value)}
                  rows={3}
                />
              </div>
            </>
          )}
          {error && (
            <p className="text-sm text-destructive">{error}</p>
          )}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t('createAgent.cancel')}
          </Button>
          <Button onClick={handleCreate} disabled={creating}>
            {creating ? t('createAgent.creating') : t('createAgent.create')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
