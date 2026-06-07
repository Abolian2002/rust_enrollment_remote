"use client";

import type { ReactNode } from "react";

import { cn } from "@/lib/cn";

type FormattedMessageProps = {
  className?: string;
  text: string;
};

export function FormattedMessage({ className, text }: FormattedMessageProps) {
  return <p className={cn("whitespace-pre-wrap", className)}>{renderLightMarkdown(text)}</p>;
}

function renderLightMarkdown(text: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  const lines = text.split("\n");

  lines.forEach((line, lineIndex) => {
    if (lineIndex > 0) {
      nodes.push(<br key={`br-${lineIndex}`} />);
    }

    const parts = line.split(/(\*\*[^*]+\*\*)/g);
    parts.forEach((part, partIndex) => {
      const key = `${lineIndex}-${partIndex}`;
      if (part.startsWith("**") && part.endsWith("**") && part.length > 4) {
        nodes.push(
          <strong key={key} className="font-bold">
            {part.slice(2, -2)}
          </strong>
        );
      } else if (part) {
        nodes.push(part);
      }
    });
  });

  return nodes;
}
