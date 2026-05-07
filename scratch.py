import re

with open("site/index.html", "r") as f:
    site = f.read()

# Fix DOCTYPE
site = site.replace("<!doctype html>", "<!DOCTYPE html>")

# Fix self-closing meta, link, img
site = re.sub(r'(<(?:meta|link|img)[^>]*?)\s*/>', r'\1>', site)

# Fix "photo" in alt attributes
site = site.replace('alt="Photos page light"', 'alt="Library page light mode"')
site = site.replace('alt="Photos page dark"', 'alt="Library page dark mode"')

# Rewrite figure as button to fix tabIndex warning
site = re.sub(r'<figure class="screenshot-item" tabindex="0">', r'<button type="button" class="screenshot-item">', site)
site = site.replace('</figure>', '</button>')
site = site.replace('<figcaption>', '<span class="figcaption">')
site = site.replace('</figcaption>', '</span>')

# Rewrite modal as <dialog>
old_modal = """    <div id="image-modal" class="modal" aria-hidden="true" role="dialog">
      <button id="modal-close" class="modal-close" aria-label="Close image">×</button>
      <div class="modal-content">
        <img id="modal-img" src="" alt="" >
        <div id="modal-caption" class="modal-caption"></div>
      </div>
    </div>"""

new_modal = """    <dialog id="image-modal" class="modal">
      <button id="modal-close" class="modal-close" aria-label="Close image" type="button">×</button>
      <div class="modal-content">
        <img id="modal-img" alt="Fullscreen screenshot view" >
        <div id="modal-caption" class="modal-caption"></div>
      </div>
    </dialog>"""

site = site.replace(old_modal, new_modal)

# Also update the JS to use dialog.showModal() and dialog.close()
js_old_1 = """          modal.classList.add("visible");
          modal.setAttribute("aria-hidden", "false");
          document.body.style.overflow = "hidden"; // Prevent background scrolling"""
js_new_1 = """          modal.showModal();
          modal.classList.add("visible");
          document.body.style.overflow = "hidden";"""
site = site.replace(js_old_1, js_new_1)

js_old_2 = """      const closeModal = () => {
        modal.classList.remove("visible");
        modal.setAttribute("aria-hidden", "true");
        document.body.style.overflow = "";
      };"""
js_new_2 = """      const closeModal = () => {
        modal.classList.remove("visible");
        modal.close();
        document.body.style.overflow = "";
      };"""
site = site.replace(js_old_2, js_new_2)

# Fix JS querySelectorAll for screenshot-item since it's a button now
site = site.replace('const figure = img.parentElement;', 'const figure = img.closest(".screenshot-item");')

with open("site/index.html", "w") as f:
    f.write(site)
