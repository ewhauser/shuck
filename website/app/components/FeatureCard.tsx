import type { ReactNode } from "react";

interface FeatureCardProps {
  icon: ReactNode;
  title: string;
  description: string;
}

export default function FeatureCard({ icon, title, description }: FeatureCardProps) {
  return (
    <div className="group relative rounded-lg border border-fg-dim/20 bg-bg-card/50 p-5 transition-all hover:border-accent/30 hover:shadow-[0_0_20px_rgba(51,255,51,0.04)]">
      <div className="mb-3 flex h-9 w-9 items-center justify-center rounded-md bg-accent/8 text-accent">
        {icon}
      </div>
      <h3 className="font-heading text-base font-semibold text-fg-primary mb-1.5">
        {title}
      </h3>
      <p className="text-sm text-fg-secondary leading-relaxed">
        {description}
      </p>
    </div>
  );
}
