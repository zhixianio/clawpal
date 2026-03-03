import { useState } from "react";
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";

interface AsyncActionButtonProps {
  onClick: () => Promise<void>;
  children: ReactNode;
  loadingText?: string;
  variant?: "default" | "outline" | "secondary" | "destructive" | "ghost" | "link";
  size?: "default" | "sm" | "lg" | "icon";
  disabled?: boolean;
  className?: string;
}

export function AsyncActionButton({
  onClick,
  children,
  loadingText,
  variant = "default",
  size = "default",
  disabled = false,
  className,
}: AsyncActionButtonProps) {
  const [isLoading, setIsLoading] = useState(false);

  const handleClick = async () => {
    if (isLoading || disabled) return;
    setIsLoading(true);
    try {
      await onClick();
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <Button
      type="button"
      variant={variant}
      size={size}
      disabled={disabled || isLoading}
      className={className}
      onClick={() => {
        void handleClick();
      }}
    >
      {isLoading && (
        <span className="h-3.5 w-3.5 animate-spin rounded-full border-2 border-current border-t-transparent" />
      )}
      <span>{isLoading ? (loadingText ?? children) : children}</span>
    </Button>
  );
}
