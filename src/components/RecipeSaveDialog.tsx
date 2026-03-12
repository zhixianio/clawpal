import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export function RecipeSaveDialog({
  open,
  title,
  confirmLabel,
  initialSlug,
  busy,
  onOpenChange,
  onConfirm,
}: {
  open: boolean;
  title: string;
  confirmLabel: string;
  initialSlug: string;
  busy: boolean;
  onOpenChange: (open: boolean) => void;
  onConfirm: (slug: string) => void;
}) {
  const { t } = useTranslation();
  const [slug, setSlug] = useState(initialSlug);

  useEffect(() => {
    if (open) {
      setSlug(initialSlug);
    }
  }, [initialSlug, open]);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{title}</DialogTitle>
        </DialogHeader>
        <div className="space-y-2">
          <Label htmlFor="recipe-save-slug">{t("recipeStudio.slugLabel")}</Label>
          <Input
            id="recipe-save-slug"
            value={slug}
            onChange={(event) => setSlug(event.target.value)}
            placeholder={t("recipeStudio.slugPlaceholder")}
          />
          <p className="text-xs text-muted-foreground">
            {t("recipeStudio.saveDialogDescription")}
          </p>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)} disabled={busy}>
            {t("config.cancel")}
          </Button>
          <Button onClick={() => onConfirm(slug)} disabled={busy || !slug.trim()}>
            {busy ? t("config.applying") : confirmLabel}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
