import * as LabelPrimitive from "@radix-ui/react-label";
import type * as React from "react";
import { cn } from "../../lib/utils";

function Label({ className, ...props }: React.ComponentProps<typeof LabelPrimitive.Root>) {
  return (
    <LabelPrimitive.Root
      data-slot="label"
      className={cn("text-sm font-medium text-muted-foreground", className)}
      {...props}
    />
  );
}

export { Label };
