"""插件与启动脚本共享真实应用标签和已声明的原生操作。"""

class _ApplicationTab:
    def __init__(self, application, identity):
        self._application, self._identity = application, identity

    @property
    def editorSession(self):
        if self._identity not in self._application._sessions:
            raise AttributeError("此标签不含本次请求中的实际编辑会话")
        return self._application._sessions[self._identity]


class _ApplicationTabs:
    def __init__(self, application):
        self._application = application

    def count(self):
        return len(self._application._tabs)

    def widget(self, index):
        if type(index) is not int:
            raise TypeError("标签索引需要整数")
        tabs = self._application._tabs
        return tabs[index] if 0 <= index < len(tabs) else None

    def currentWidget(self):
        return self._application.currentTab()

    def currentIndex(self):
        return self.indexOf(self.currentWidget())

    def indexOf(self, tab):
        return next((index for index, current in enumerate(self._application._tabs) if current is tab), -1)

    def setCurrentWidget(self, tab):
        if self.indexOf(tab) < 0:
            raise ValueError("目标标签不属于当前真实应用目录")
        self._application._call("activate", identity=tab._identity)

    def setCurrentIndex(self, index):
        tab = self.widget(index)
        if tab is None:
            raise IndexError("应用标签索引超出范围")
        self.setCurrentWidget(tab)

    def tabBar(self):
        return self

    def moveTab(self, source, destination):
        tab, anchor = self.widget(source), self.widget(destination)
        if tab is None or anchor is None:
            raise IndexError("移动标签索引超出实际范围")
        self._application._call("move", identity=tab._identity, anchor=anchor._identity, after=source < destination)


class _ApplicationActions:
    def _init_application(self):
        self.tabWidget = _ApplicationTabs(self)
        self._tabs = []

    def _bind_tabs(self, directory):
        previous = {tab._identity: tab for tab in self._tabs}
        self._tabs = [previous[tab["identity"]] if tab["identity"] in previous else
                      _ApplicationTab(self, tab["identity"]) for tab in directory["tabs"]]
        self._active_tab = directory["active_tab"]
        for tab in self._tabs:
            if tab._identity in self._sessions:
                self._sessions[tab._identity].editorTab = tab

    def _bind_editor_session(self, identity, session):
        self._sessions[identity] = session
        session.editorTab = next(tab for tab in self._tabs if tab._identity == identity)

    def _apply_application_reply(self, reply):
        if reply["loaded"] is not None:
            self._bind_loaded_document(reply["loaded"])
        self._bind_tabs(reply["directory"])

    @property
    def sessions(self):
        return [self._sessions[tab._identity] for tab in self._tabs
                if tab._identity in self._sessions]

    def currentSession(self):
        return self._sessions.get(self._active_tab)

    def currentTab(self):
        return next((tab for tab in self._tabs if tab._identity == self._active_tab), None)

    def saveCurrentWorld(self):
        session = self.currentSession()
        if session is not None:
            session.save()

    def loadFile(self, filename, readonly=False):
        if not isinstance(filename, str):
            raise TypeError("打开文件需要实际路径字符串")
        self._call("open", path=filename, readonly=bool(readonly))

    def tabCloseRequested(self, index):
        tab = self.tabWidget.widget(index)
        if tab is None:
            raise IndexError("关闭标签索引超出实际范围")
        self._call("close", identity=tab._identity)

    def closeCurrentTab(self):
        tab = self.currentTab()
        if tab is not None:
            self._call("close", identity=tab._identity)

    def exitEditor(self):
        self._call("exit")

    def showWorldList(self):
        self._call("world_list", visible=True)

    def hideWorldList(self):
        self._call("world_list", visible=False)

