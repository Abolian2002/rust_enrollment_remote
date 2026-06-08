import { cn } from "@/lib/cn";

type FormattedMessageProps = {
  className?: string;
  text: string;
};

export function FormattedMessage({ className, text }: FormattedMessageProps) {
  return (
    <p className={cn("whitespace-pre-wrap break-words [line-break:strict]", className)}>
      {formatDisplayText(text)}
    </p>
  );
}

export function formatDisplayText(text: string) {
  return text
    .replace(/\*\*([^*\n]+)\*\*/g, "$1")
    .replace(/\*\*/g, "")
    .replace(/(^|\n)\s*[*+-]\s+/g, "$1• ")
    .replace(/(^|\n)\s{0,3}#{1,6}\s*/g, "$1")
    .replace(/`([^`\n]+)`/g, "$1")
    .replace(/```[\s\S]*?```/g, (block) => block.replace(/```[a-zA-Z0-9_-]*\n?/g, "").replace(/```/g, ""));
}
