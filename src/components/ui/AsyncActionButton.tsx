import { forwardRef, useState } from "react";
import type { ReactNode, ComponentPropsWithoutRef } from "react";
import { Button } from "@/components/ui/button";

interface AsyncActionButtonProps
  extends Omit<ComponentPropsWithoutRef<typeof Button>, "onClick"> {
  onClick: () => Promise<void>;
  children: ReactNode;
  loadingText?: string;
}

export const AsyncActionButton = forwardRef<HTMLButtonElement, AsyncActionButtonProps>(
  function AsyncActionButton(
    { onClick, children, loadingText, disabled = false, ...rest },
    ref,
  ) {
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
        ref={ref}
        type="button"
        disabled={disabled || isLoading}
        {...rest}
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
  },
);
