use super::*;

impl AgentWorkbench {
    pub(super) fn position_active_diff(
        &self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let selected_hunk = self
            .tab_index(path)
            .and_then(|index| self.tabs[index].diff.as_ref())
            .and_then(|diff| {
                diff.editor
                    .clone()
                    .zip(diff.hunk_lines.get(diff.selected_hunk).copied())
            });
        if let Some((editor, line)) = selected_hunk {
            set_diff_cursor(&editor, line, window, cx);
        }
    }

    pub(super) fn tab_index(&self, path: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.path == path)
    }

    pub(super) fn activate_tab(&mut self, path: String, cx: &mut Context<'_, Self>) {
        if self.tab_index(&path).is_none() {
            return;
        }
        self.active_path = Some(path);
        self.sync_tree_selection(cx);
        cx.notify();
    }

    pub(super) fn sync_tree_selection(&self, cx: &mut Context<'_, Self>) {
        let selected: Option<SharedString> = self.active_path.clone().map(Into::into);
        self.tree_state.update(cx, |state, cx| {
            let index = selected.as_ref().and_then(|path| state.index_of(path));
            state.set_selected_index(index, cx);
        });
    }

    pub(super) fn close_tab(&mut self, path: &str, cx: &mut Context<'_, Self>) {
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let was_active = self.active_path.as_deref() == Some(path);
        self.tabs.remove(index);
        if was_active {
            self.active_path = self
                .tabs
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|index| self.tabs.get(index)))
                .map(|tab| tab.path.clone());
        }
        self.sync_tree_selection(cx);
        cx.notify();
    }

    pub(super) fn close_active_tab(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(path) = self.active_path.clone() {
            self.close_tab(&path, cx);
        }
    }

    pub(super) fn cycle_tab(&mut self, offset: isize, cx: &mut Context<'_, Self>) {
        if self.tabs.len() < 2 {
            return;
        }
        let current = self
            .active_path
            .as_deref()
            .and_then(|path| self.tab_index(path))
            .unwrap_or_default();
        let len = isize::try_from(self.tabs.len()).expect("tab count fits in isize");
        let index = (isize::try_from(current).expect("tab index fits in isize") + offset)
            .rem_euclid(len) as usize;
        self.activate_tab(self.tabs[index].path.clone(), cx);
    }

    pub(super) fn toggle_explorer(&mut self, cx: &mut Context<'_, Self>) {
        self.explorer_open = !self.explorer_open;
        cx.notify();
    }

    pub(super) fn set_view_mode(
        &mut self,
        mode: WorkbenchViewMode,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(path) = self.active_path.as_deref() else {
            return;
        };
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let tab = &mut self.tabs[index];
        if mode == WorkbenchViewMode::Diff
            && !tab.diff.as_ref().is_some_and(WorkbenchDiff::has_content)
        {
            return;
        }
        if tab.mode != mode {
            tab.mode = mode;
            let editor = if mode == WorkbenchViewMode::Diff {
                tab.diff
                    .as_ref()
                    .and_then(|diff| diff.editor.clone())
                    .unwrap_or_else(|| tab.editor.clone())
            } else {
                tab.editor.clone()
            };
            editor.update(cx, |editor, cx| editor.focus(window, cx));
            cx.notify();
        }
    }

    pub(super) fn toggle_view_mode(&mut self, window: &mut Window, cx: &mut Context<'_, Self>) {
        let Some(path) = self.active_path.as_deref() else {
            return;
        };
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let mode = match self.tabs[index].mode {
            WorkbenchViewMode::File => WorkbenchViewMode::Diff,
            WorkbenchViewMode::Diff => WorkbenchViewMode::File,
        };
        self.set_view_mode(mode, window, cx);
    }

    pub(super) fn navigate_hunk(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<'_, Self>,
    ) {
        let Some(path) = self.active_path.as_deref() else {
            return;
        };
        let Some(index) = self.tab_index(path) else {
            return;
        };
        let tab = &mut self.tabs[index];
        let Some(diff) = tab.diff.as_mut() else {
            return;
        };
        if tab.mode != WorkbenchViewMode::Diff || diff.hunk_lines.is_empty() {
            return;
        }
        let next = if direction.is_positive() {
            diff.selected_hunk
                .checked_add(1)
                .filter(|next| *next < diff.hunk_lines.len())
        } else {
            diff.selected_hunk.checked_sub(1)
        };
        let Some(next) = next else {
            return;
        };
        let line = diff.hunk_lines[next];
        diff.selected_hunk = next;
        let Some(editor) = diff.editor.clone() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.set_cursor_position(
                Position {
                    line: u32::try_from(line).unwrap_or(u32::MAX),
                    character: 0,
                },
                window,
                cx,
            );
        });
        cx.notify();
    }
}
