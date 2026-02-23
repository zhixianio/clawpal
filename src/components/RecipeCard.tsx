import { useTranslation } from "react-i18next";
import type { Recipe } from "../lib/types";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";

export function RecipeCard({
  recipe,
  onCook,
  compact,
}: {
  recipe: Recipe;
  onCook: (id: string) => void;
  compact?: boolean;
}) {
  const { t } = useTranslation();

  if (compact) {
    return (
      <Card
        className="cursor-pointer hover:border-primary/40 hover:shadow-[var(--shadow-warm-hover)] transition-all duration-300 group"
        onClick={() => onCook(recipe.id)}
      >
        <CardContent>
          <strong className="group-hover:text-primary transition-colors duration-200">{recipe.name}</strong>
          <div className="text-sm text-muted-foreground mt-1.5 line-clamp-2">
            {recipe.description}
          </div>
          <div className="text-xs text-muted-foreground/70 mt-2.5 flex items-center gap-1.5">
            <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded-full">
              {t('recipeCard.steps', { count: recipe.steps.length })}
            </span>
            <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded-full">
              {t(`recipeCard.${recipe.difficulty}`)}
            </span>
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card className="group">
      <CardHeader>
        <CardTitle>{recipe.name}</CardTitle>
        <CardDescription>{recipe.description}</CardDescription>
      </CardHeader>
      <CardContent>
        <div className="flex flex-wrap gap-1.5 mb-3">
          {recipe.tags.map((tag) => (
            <Badge key={tag} variant="secondary" className="bg-primary/8 text-primary/80 border-0">
              {tag}
            </Badge>
          ))}
        </div>
        <p className="text-sm text-muted-foreground flex items-center gap-2">
          <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded-full text-xs">
            {t('recipeCard.steps', { count: recipe.steps.length })}
          </span>
          <span className="inline-flex items-center gap-1 bg-muted px-2 py-0.5 rounded-full text-xs">
            {t(`recipeCard.${recipe.difficulty}`)}
          </span>
        </p>
      </CardContent>
      <CardFooter>
        <Button onClick={() => onCook(recipe.id)}>
          {t('recipeCard.cook')}
        </Button>
      </CardFooter>
    </Card>
  );
}
