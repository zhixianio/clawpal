import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../lib/api";
import { RecipeCard } from "../components/RecipeCard";
import type { Recipe } from "../lib/types";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { AsyncActionButton } from "@/components/ui/AsyncActionButton";

export function Recipes({
  onCook,
}: {
  onCook: (id: string, source?: string) => void;
}) {
  const { t } = useTranslation();
  const [recipes, setRecipes] = useState<Recipe[]>([]);
  const [source, setSource] = useState("");
  const [loadedSource, setLoadedSource] = useState<string | undefined>(undefined);

  const load = async (nextSource: string) => {
    const value = nextSource.trim();
    try {
      const r = await api.listRecipes(value || undefined);
      setLoadedSource(value || undefined);
      setRecipes(r);
    } catch (e) {
      console.error("Failed to load recipes:", e);
    }
  };

  useEffect(() => {
    void load("");
  }, []);

  return (
    <section>
      <h2 className="text-2xl font-bold mb-4">{t('recipes.title')}</h2>
      <div className="mb-2 flex items-center gap-2">
        <Label>{t('recipes.sourceLabel')}</Label>
        <Input
          value={source}
          onChange={(event) => setSource(event.target.value)}
          placeholder="/path/recipes.json or https://example.com/recipes.json"
          className="w-[380px]"
        />
        <AsyncActionButton className="ml-2" onClick={() => load(source)} loadingText={t('recipes.loading')}>
          {t('recipes.load')}
        </AsyncActionButton>
      </div>
      <p className="text-sm text-muted-foreground mt-0">
        {t('recipes.loadedFrom', { source: loadedSource || t('recipes.builtinSource') })}
      </p>
      <div className="grid grid-cols-[repeat(auto-fit,minmax(220px,1fr))] gap-3">
        {recipes.map((recipe) => (
          <RecipeCard
            key={recipe.id}
            recipe={recipe}
            onCook={() => onCook(recipe.id, loadedSource)}
          />
        ))}
      </div>
    </section>
  );
}
