import { Header } from "../components/Header";
import { Hero } from "../components/Hero";
import { Proof } from "../components/Proof";
import { Features } from "../components/Features";
import { AppShot } from "../components/AppShot";
import { Install } from "../components/Install";
import { Thanks } from "../components/Thanks";
import { Footer } from "../components/Footer";

export default function Page() {
  return (
    <>
      <Header />
      <main className="site-main">
        <Hero />
        <Proof />
        <Features />
        <AppShot />
        <Install />
        <Thanks />
      </main>
      <Footer />
    </>
  );
}
