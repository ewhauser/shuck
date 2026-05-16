import Header from "@/app/components/Header";
import Footer from "@/app/components/Footer";
import DocsSidebar from "@/app/components/docs/DocsSidebar";
import { Suspense } from "react";

export default function DocsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <>
      <Header />
      <div className="mx-auto flex max-w-6xl">
        <Suspense>
          <DocsSidebar />
        </Suspense>
        <main className="min-w-0 flex-1 px-4 sm:px-8 py-8 lg:py-10">
          {children}
        </main>
      </div>
      <Footer />
    </>
  );
}
