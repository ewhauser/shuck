import Link from "next/link";
import Header from "./components/Header";
import Footer from "./components/Footer";
import FeatureCard from "./components/FeatureCard";
import { withBasePath } from "./lib/site";
import { Zap, RefreshCw, Plug, Terminal } from "lucide-react";

export default function Home() {
  return (
    <>
      <Header />

      {/* Hero */}
      <section className="mx-auto max-w-6xl px-4 sm:px-6 pt-12 pb-10 lg:pt-16 lg:pb-14">
        <div className="grid gap-10 lg:grid-cols-2 lg:gap-14 items-center">
          {/* Left: info */}
          <div>
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={withBasePath("/logo.svg")}
              alt="shuck"
              className="h-28 w-28 sm:h-36 sm:w-36 mb-5"
            />
            <p className="text-base text-fg-secondary leading-relaxed mb-6">
              A fast shell script linter, built in Rust. Checks for
              correctness, portability, and style issues in your shell scripts,
              with a built-in LSP server for editor integration.
            </p>

            {/* Install */}
            <div className="space-y-2.5 mb-6">
              <div>
                <p className="text-xs text-fg-dim mb-1 font-medium">Install</p>
                <pre className="rounded-md bg-bg-secondary border border-fg-dim/20 px-3 py-2 text-sm font-mono text-fg-primary overflow-x-auto">
                  cargo install shuck-cli
                </pre>
              </div>
              <div>
                <p className="text-xs text-fg-dim mb-1 font-medium">Usage</p>
                <pre className="rounded-md bg-bg-secondary border border-fg-dim/20 px-3 py-2 text-sm font-mono text-fg-primary overflow-x-auto">
                  shuck check .
                </pre>
              </div>
              <div>
                <p className="text-xs text-fg-dim mb-1 font-medium">Editor LSP</p>
                <pre className="rounded-md bg-bg-secondary border border-fg-dim/20 px-3 py-2 text-sm font-mono text-fg-primary overflow-x-auto">
                  shuck server
                </pre>
              </div>
            </div>

            {/* Links */}
            <div className="flex flex-wrap gap-x-6 gap-y-2 text-sm">
              <Link
                href="/docs/getting-started"
                className="text-accent hover:underline"
              >
                Docs
              </Link>
              <Link
                href="/docs/editors"
                className="text-accent hover:underline"
              >
                Editor Setup
              </Link>
              <Link
                href="/docs/performance/benchmarks"
                className="text-accent hover:underline"
              >
                Benchmarks
              </Link>
              <a
                href="https://github.com/ewhauser/shuck"
                className="text-accent hover:underline"
                target="_blank"
                rel="noopener noreferrer"
              >
                GitHub
              </a>
              <a
                href="https://github.com/ewhauser/shuck/releases"
                className="text-accent hover:underline"
                target="_blank"
                rel="noopener noreferrer"
              >
                Releases
              </a>
            </div>
          </div>

          {/* Right: sample output */}
          <div className="rounded-xl border border-fg-dim/20 bg-[#0c0d0e] overflow-hidden shadow-[0_25px_50px_rgba(0,0,0,0.4)]">
            {/* Title bar */}
            <div className="flex items-center h-9 px-3.5 bg-[#141516] border-b border-white/[0.08] gap-2">
              <div className="w-3 h-3 rounded-full bg-[#ff5f57]" />
              <div className="w-3 h-3 rounded-full bg-[#febc2e]" />
              <div className="w-3 h-3 rounded-full bg-[#28c840]" />
              <span className="flex-1 text-center text-[11px] font-mono font-medium text-fg-dim tracking-wider">
                shuck check --select S deploy.sh
              </span>
            </div>
            {/* Output */}
            <div className="p-4 font-mono text-[12px] leading-[1.5] overflow-x-auto whitespace-pre text-fg-dim">
              <div><span className="text-[#febc2e]">warning[S001]</span><span className="text-fg-primary">: quote parameter expansions to avoid word splitting and globbing</span></div>
              <div><span className="text-accent">{"  --> "}</span><span className="text-fg-secondary">deploy.sh:3:10</span></div>
              <div><span className="text-accent">{"   |"}</span></div>
              <div><span className="text-accent">{"3  |"}</span><span className="text-fg-secondary">{"  rm -rf $"}</span><span className="text-[#febc2e]">build_dir</span></div>
              <div><span className="text-accent">{"   |"}</span>{"          "}<span className="text-[#febc2e]">{"^^^^^^^^^^"}</span></div>
              <div><span className="text-accent">{"   |"}</span></div>
              <div>&nbsp;</div>
              <div><span className="text-[#febc2e]">warning[S014]</span><span className="text-fg-primary">: quote star-splat expansions to preserve argument boundaries</span></div>
              <div><span className="text-accent">{"  --> "}</span><span className="text-fg-secondary">deploy.sh:7:12</span></div>
              <div><span className="text-accent">{"   |"}</span></div>
              <div><span className="text-accent">{"7  |"}</span><span className="text-fg-secondary">{"  for arg in $"}</span><span className="text-[#febc2e]">*</span><span className="text-fg-secondary">{"; do"}</span></div>
              <div><span className="text-accent">{"   |"}</span>{"             "}<span className="text-[#febc2e]">{"^^"}</span></div>
              <div><span className="text-accent">{"   |"}</span></div>
              <div>&nbsp;</div>
              <div><span className="text-[#febc2e]">warning[S005]</span><span className="text-fg-primary">{": prefer `$(...)` over legacy backtick substitution"}</span></div>
              <div><span className="text-accent">{"  --> "}</span><span className="text-fg-secondary">deploy.sh:12:10</span></div>
              <div><span className="text-accent">{"   |"}</span></div>
              <div><span className="text-accent">{"12 |"}</span><span className="text-fg-secondary">{"  local v="}</span><span className="text-[#febc2e]">{"`git describe`"}</span></div>
              <div><span className="text-accent">{"   |"}</span>{"          "}<span className="text-[#febc2e]">{"^^^^^^^^^^^^^^^"}</span></div>
              <div><span className="text-accent">{"   |"}</span></div>
            </div>
          </div>
        </div>
      </section>

      {/* Features */}
      <section className="mx-auto max-w-6xl px-4 sm:px-6 pb-16">
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
          <FeatureCard
            icon={<Zap className="h-5 w-5" />}
            title="Fast"
            description="Built in Rust with per-file caching. Lints thousands of shell scripts in seconds."
          />
          <FeatureCard
            icon={<RefreshCw className="h-5 w-5" />}
            title="Compatible"
            description="Supports ShellCheck suppression directives and SC codes. Migrate incrementally without rewriting your existing inline annotations."
          />
          <FeatureCard
            icon={<Terminal className="h-5 w-5" />}
            title="Multi-Shell"
            description="Lint bash, sh, dash, ksh, mksh, and zsh scripts. Real parser support for each dialect, not just regex matching."
          />
          <FeatureCard
            icon={<Plug className="h-5 w-5" />}
            title="Integrated"
            description="Works with your editor through the built-in LSP server, plus CI pipelines and pre-commit hooks. One binary, zero dependencies."
          />
        </div>
      </section>

      <Footer />
    </>
  );
}
