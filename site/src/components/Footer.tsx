const REPO = "https://github.com/iAmChumby/canvas-rgb";

export function Footer() {
  return (
    <footer className="site-footer">
      <div className="site-footer-inner">
        <p>
          Canvas RGB is Apache-2.0.{" "}
          <a href={`${REPO}/blob/main/NOTICE`}>NOTICE</a> lists the code and research it builds
          on.
        </p>
        <p>Built by Luke.</p>
      </div>
    </footer>
  );
}
