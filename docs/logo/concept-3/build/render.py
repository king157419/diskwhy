"""Rasterise SVG/HTML with Playwright driving installed Edge (channel=msedge)."""
import pathlib
from playwright.sync_api import sync_playwright


class Renderer:
    def __enter__(self):
        self._pw = sync_playwright().start()
        self.browser = self._pw.chromium.launch(channel="msedge")
        self.ctx = self.browser.new_context(device_scale_factor=1)
        self.page = self.ctx.new_page()
        return self

    def __exit__(self, *a):
        self.browser.close()
        self._pw.stop()

    def svg(self, svg_path, size, out_png, w=None):
        """Render an SVG file to a transparent PNG at exactly size x size (or w x size)."""
        w = w or size
        uri = pathlib.Path(svg_path).resolve().as_uri()
        html = (f"<html><body style='margin:0;background:transparent'>"
                f"<img id=i src='{uri}' style='display:block;width:{w}px;height:{size}px'></body></html>")
        self.page.set_viewport_size({"width": max(w, 64), "height": max(size, 64)})
        tmp = pathlib.Path(svg_path).resolve().parent / "_render_tmp.html"
        tmp.write_text(html, encoding="utf8")
        self.page.goto(tmp.as_uri())
        self.page.wait_for_function("document.getElementById('i').complete && document.getElementById('i').naturalWidth>0")
        tmp_cleanup = tmp
        self.page.screenshot(path=str(out_png), clip={"x": 0, "y": 0, "width": w, "height": size},
                             omit_background=True)
        tmp_cleanup.unlink(missing_ok=True)

    def html(self, html_path, out_png, width=1600):
        self.page.set_viewport_size({"width": width, "height": 800})
        self.page.goto(pathlib.Path(html_path).resolve().as_uri())
        self.page.wait_for_load_state("networkidle")
        self.page.evaluate("document.fonts.ready")
        self.page.screenshot(path=str(out_png), full_page=True)
