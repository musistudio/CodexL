use super::CdpTarget;

pub(super) struct KeyEvent {
    pub(super) code: &'static str,
    pub(super) key: &'static str,
    pub(super) windows_virtual_key_code: u16,
}

pub(super) fn select_target(targets: &[CdpTarget]) -> Option<CdpTarget> {
    let page_targets: Vec<&CdpTarget> = targets
        .iter()
        .filter(|target| !target.web_socket_debugger_url.is_empty() && target.target_type == "page")
        .collect();
    page_targets
        .iter()
        .find(|target| {
            format!("{} {}", target.title, target.url)
                .to_lowercase()
                .contains("codex")
        })
        .copied()
        .cloned()
        .or_else(|| page_targets.first().copied().cloned())
        .or_else(|| {
            targets
                .iter()
                .find(|target| !target.web_socket_debugger_url.is_empty())
                .cloned()
        })
}

pub(super) fn key_event_for(key: &str) -> KeyEvent {
    match key {
        "Backspace" => KeyEvent {
            code: "Backspace",
            key: "Backspace",
            windows_virtual_key_code: 8,
        },
        "Tab" => KeyEvent {
            code: "Tab",
            key: "Tab",
            windows_virtual_key_code: 9,
        },
        "Enter" => KeyEvent {
            code: "Enter",
            key: "Enter",
            windows_virtual_key_code: 13,
        },
        "Escape" => KeyEvent {
            code: "Escape",
            key: "Escape",
            windows_virtual_key_code: 27,
        },
        "Delete" => KeyEvent {
            code: "Delete",
            key: "Delete",
            windows_virtual_key_code: 46,
        },
        "ArrowLeft" => KeyEvent {
            code: "ArrowLeft",
            key: "ArrowLeft",
            windows_virtual_key_code: 37,
        },
        "ArrowUp" => KeyEvent {
            code: "ArrowUp",
            key: "ArrowUp",
            windows_virtual_key_code: 38,
        },
        "ArrowRight" => KeyEvent {
            code: "ArrowRight",
            key: "ArrowRight",
            windows_virtual_key_code: 39,
        },
        "ArrowDown" => KeyEvent {
            code: "ArrowDown",
            key: "ArrowDown",
            windows_virtual_key_code: 40,
        },
        _ => KeyEvent {
            code: "",
            key: "",
            windows_virtual_key_code: 0,
        },
    }
}

pub(super) fn editable_probe_expression(x: f64, y: f64) -> String {
    format!(
        r#"(() => {{
          {}
          let element = document.elementFromPoint({}, {});
          while (element && element.shadowRoot) {{
            const nested = element.shadowRoot.elementFromPoint({}, {});
            if (!nested || nested === element) break;
            element = nested;
          }}
          return closestEditable(element);
        }})()"#,
        editable_helpers(),
        x,
        y,
        x,
        y
    )
}

pub(super) fn editable_focus_expression() -> String {
    format!(
        r#"(() => {{
          {}
          let active = document.activeElement;
          while (active && active.shadowRoot && active.shadowRoot.activeElement) {{
            active = active.shadowRoot.activeElement;
          }}
          return closestEditable(active);
        }})()"#,
        editable_helpers()
    )
}

pub(super) fn scrollable_probe_expression(x: f64, y: f64) -> String {
    format!(
        r#"(() => {{
          {}
          let element = document.elementFromPoint({}, {});
          while (element && element.shadowRoot) {{
            const nested = element.shadowRoot.elementFromPoint({}, {});
            if (!nested || nested === element) break;
            element = nested;
          }}
          return closestScrollable(element);
        }})()"#,
        scrollable_helpers(),
        x,
        y,
        x,
        y
    )
}

pub(super) fn set_sidebar_expression(side: &str, action: &str) -> String {
    format!(
        r#"(() => {{
          const side = {};
          const action = {};
          const viewportWidth = Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1);
          const viewportHeight = Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1);
          const selector = "button, [role='button'], [aria-label][tabindex], [title][tabindex]";
          const commonWords = ["sidebar", "side bar", "side panel", "侧边栏", "边栏"];
          const leftWords = ["left", "primary", "nav", "navigation", "activity", "history", "project", "projects", "workspace", "左", "左侧", "项目", "对话"];
          const rightWords = ["right", "secondary", "auxiliary", "details", "detail", "panel", "inspector", "右", "右侧", "面板", "详情"];
          const closeWords = ["close", "hide", "collapse", "关闭", "隐藏", "收起"];
          const openWords = ["open", "show", "expand", "toggle", "打开", "显示", "展开", "切换"];

          function text(value) {{
            return String(value || "").replace(/\s+/g, " ").trim();
          }}

          function labelFor(element) {{
            return [
              element.getAttribute("aria-label"),
              element.getAttribute("title"),
              element.getAttribute("data-testid"),
              element.getAttribute("data-test-id"),
              element.id,
              element.textContent,
            ].map(text).filter(Boolean).join(" ").toLowerCase();
          }}

          function includesAny(label, words) {{
            return words.some((word) => label.includes(word));
          }}

          function visibleRect(element) {{
            const style = window.getComputedStyle(element);
            if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) return null;
            const rect = element.getBoundingClientRect();
            const left = Math.max(0, rect.left);
            const top = Math.max(0, rect.top);
            const right = Math.min(viewportWidth, rect.right);
            const bottom = Math.min(viewportHeight, rect.bottom);
            if (right <= left || bottom <= top || right - left < 8 || bottom - top < 8) return null;
            return {{ left, top, right, bottom, width: right - left, height: bottom - top }};
          }}

          function expandedState(element) {{
            const value = element.getAttribute("aria-expanded") ?? element.getAttribute("aria-pressed");
            if (value === "true") return true;
            if (value === "false") return false;
            return null;
          }}

          function isSideOpen() {{
            for (const element of document.querySelectorAll(selector)) {{
              const rect = visibleRect(element);
              if (!rect) continue;
              const centerX = rect.left + rect.width / 2;
              const inZone = side === "left" ? centerX <= viewportWidth * 0.4 : centerX >= viewportWidth * 0.6;
              if (!inZone) continue;
              const label = labelFor(element);
              const words = side === "left" ? leftWords : rightWords;
              if ((includesAny(label, words) || includesAny(label, commonWords)) && expandedState(element) === true) {{
                return true;
              }}
            }}
            return false;
          }}

          const roots = [document];
          const candidates = [];
          for (let index = 0; index < roots.length; index += 1) {{
            const root = roots[index];
            for (const element of root.querySelectorAll(selector)) {{
              if (element.shadowRoot) roots.push(element.shadowRoot);
              const rect = visibleRect(element);
              if (!rect) continue;
              const centerX = rect.left + rect.width / 2;
              const centerY = rect.top + rect.height / 2;
              const sideZone = side === "left" ? centerX <= viewportWidth * 0.42 : centerX >= viewportWidth * 0.58;
              if (!sideZone) continue;
              const label = labelFor(element);
              const sideWords = side === "left" ? leftWords : rightWords;
              let score = 0;
              if (includesAny(label, sideWords)) score += 40;
              if (includesAny(label, commonWords)) score += 30;
              if (action === "close" && includesAny(label, closeWords)) score += 45;
              if (action === "open" && includesAny(label, openWords)) score += 35;
              if (expandedState(element) === (action === "close")) score += 25;
              score += Math.max(0, 30 - centerY / 8);
              if (score > 0) candidates.push({{ element, score }});
            }}
          }}

          candidates.sort((a, b) => b.score - a.score);
          const target = candidates[0]?.element;
          if (!target) {{
            return {{ ok: action === "close" && !isSideOpen(), sideOpen: isSideOpen(), reason: "no matching sidebar control" }};
          }}
          target.click();
          return {{ ok: true, clicked: true, sideOpen: isSideOpen() }};
        }})()"#,
        serde_json::to_string(side).unwrap_or_else(|_| "\"left\"".to_string()),
        serde_json::to_string(action).unwrap_or_else(|_| "\"open\"".to_string())
    )
}

fn editable_helpers() -> &'static str {
    r#"
      const nonEditableInputTypes = new Set(["button", "checkbox", "color", "file", "hidden", "image", "radio", "range", "reset", "submit"]);
      const editableRoles = new Set(["combobox", "searchbox", "textbox"]);
      function isDisabledOrReadonly(element) {
        return element.disabled || element.readOnly || element.getAttribute("aria-disabled") === "true" || element.getAttribute("aria-readonly") === "true";
      }
      function isEditableElement(element) {
        if (!element || element.nodeType !== Node.ELEMENT_NODE) return false;
        const tagName = element.localName;
        if (tagName === "textarea") return !isDisabledOrReadonly(element);
        if (tagName === "input") {
          const type = (element.getAttribute("type") || "text").toLowerCase();
          return !nonEditableInputTypes.has(type) && !isDisabledOrReadonly(element);
        }
        if (element.isContentEditable) return !isDisabledOrReadonly(element);
        const role = (element.getAttribute("role") || "").toLowerCase();
        return editableRoles.has(role) && !isDisabledOrReadonly(element);
      }
      function composedParent(element) {
        if (!element) return null;
        if (element.parentElement) return element.parentElement;
        const root = element.getRootNode && element.getRootNode();
        return root && root.host ? root.host : null;
      }
      function closestEditable(element) {
        let current = element;
        while (current) {
          if (isEditableElement(current)) return true;
          current = composedParent(current);
        }
        return false;
      }
    "#
}

fn scrollable_helpers() -> &'static str {
    r#"
      const viewportWidth = Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1);
      const viewportHeight = Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1);
      const scrollableOverflowValues = new Set(["auto", "scroll", "overlay"]);
      function composedParent(element) {
        if (!element) return null;
        if (element.parentElement) return element.parentElement;
        const root = element.getRootNode && element.getRootNode();
        return root && root.host ? root.host : null;
      }
      function canScrollAxis(scrollSize, clientSize, overflowValue) {
        return scrollSize - clientSize > 2 && scrollableOverflowValues.has(overflowValue);
      }
      function isScrollableElement(element) {
        if (!element || element.nodeType !== Node.ELEMENT_NODE) return false;
        if (element === document.documentElement || element === document.body) return false;
        const style = window.getComputedStyle(element);
        if (style.display === "none" || style.visibility === "hidden" || Number(style.opacity) === 0) return false;
        const rect = element.getBoundingClientRect();
        if (rect.width < 16 || rect.height < 16) return false;
        if (rect.width >= viewportWidth * 0.96 && rect.height >= viewportHeight * 0.96) return false;
        return canScrollAxis(element.scrollWidth, element.clientWidth, style.overflowX) ||
          canScrollAxis(element.scrollHeight, element.clientHeight, style.overflowY);
      }
      function closestScrollable(element) {
        let current = element;
        while (current) {
          if (isScrollableElement(current)) return true;
          current = composedParent(current);
        }
        return false;
      }
    "#
}
