import Header from "@/app/components/Header";
import Footer from "@/app/components/Footer";
import DocsSidebar from "@/app/components/docs/DocsSidebar";

export default function DocsLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <>
      <Header />
      <div className="mx-auto flex max-w-6xl">
        <DocsSidebar />
        <main className="min-w-0 flex-1 px-4 sm:px-8 py-8 lg:py-10">
          <div className="mdx-content max-w-3xl">{children}</div>
        </main>
      </div>
      <Footer />
    </>
  );
}
