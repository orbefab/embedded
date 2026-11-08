import { cn } from "@/lib/cn";

/**
 * The mark: a thick ring (stroke 32 on r=100 in a 320 viewBox) and an orange
 * dot (r=32) at (221.6, 81.2). The ring follows currentColor so it is ink on
 * paper and paper on ink. The dot stays #E85D04.
 */
export function Ring({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={cn("shrink-0", className)}
      fill="none"
      viewBox="0 0 320 320"
      xmlns="http://www.w3.org/2000/svg"
    >
      <circle
        cx="160"
        cy="160"
        r="100"
        stroke="currentColor"
        strokeWidth="32"
      />
      <circle cx="221.6" cy="81.2" fill="#E85D04" r="32" />
    </svg>
  );
}

/**
 * Wordmark. The ring is the letter O. The letters are Syne SemiBold and read
 * RBEFAB, so the word reads Orbefab. Proportions follow the locked lockup:
 * the ring box is 132 against a 118px letter size, pulled -4px toward the R
 * so the ring nearly touches the R without covering it.
 */
export function Wordmark({ className }: { className?: string }) {
  return (
    <span
      aria-label="Orbefab"
      className={cn(
        "inline-flex items-center font-semibold font-wordmark text-foreground leading-none tracking-[-0.03em]",
        className
      )}
      role="img"
    >
      <Ring className="size-[1.119em]" />
      <span aria-hidden="true" className="-ml-[0.034em]">
        RBEFAB
      </span>
    </span>
  );
}
