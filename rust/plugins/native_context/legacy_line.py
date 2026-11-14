"""使用旧工具类接口的连续折线；每次点击生成完整候选，保留同次激活的起点。"""
from PySide import QtGui
from mcedit2.editortools import EditorTool
from mcedit2.plugins import registerToolClass
from mcedit2.widgets.blockpicker import BlockTypeButton
from mcedit2.util.bresenham import bresenham
from mcedit2.rendering.selection import SelectionBoxNode
from mcedit2.rendering.scenegraph.matrix import Translate
from mceditlib.selection import BoundingBox


@registerToolClass
class ConnectedLineTool(EditorTool):
    name = "连续折线工具"
    modifiesWorld = True

    def __init__(self, editorSession):
        super().__init__(editorSession)
        self.dimension = editorSession.currentDimension
        self.previous = None
        self.count = 0
        self.cursorNode = SelectionBoxNode("当前端点")
        self.cursorNode.selectionBox = BoundingBox((0, 0, 0), (1, 1, 1))
        self.cursorNode.color = (1., .55, .1, .2)
        self.cursorNode.wireColor = (1., .65, .1, 1.)
        self.cursorNode.visible = False
        self.cursorPosition = Translate()
        self.cursorNode.addState(self.cursorPosition)
        self.overlayNode = SelectionBoxNode("上一端点")
        self.overlayNode.selectionBox = BoundingBox((0, 0, 0), (1, 1, 1))
        self.overlayNode.visible = False
        self.previousPosition = Translate()
        self.overlayNode.addState(self.previousPosition)
        self.toolWidget = QtGui.QWidget()
        self.toolWidget.setWindowTitle("连续折线")
        self.blockButton = BlockTypeButton()
        self.blockButton.editorSession = editorSession
        self.blockButton.block = editorSession.currentDimension.blocktypes["minecraft:stone"]
        self.status = QtGui.QLabel("尚未选择起点")
        self.resetButton = QtGui.QPushButton("重新开始")
        self.resetButton.clicked.connect(self.reset)
        self.showCursorAction = QtGui.QAction("显示悬停端点", self.toolWidget, checkable=True, checked=True)
        self.showCursorAction.toggled.connect(self.cursorVisibility)
        self.selectToolAction = QtGui.QAction("切换到选择工具", self.toolWidget,
                                             triggered=lambda: self.editorSession.chooseTool("Select"))
        self.toolWidget.addActions([self.showCursorAction, self.selectToolAction])
        form = QtGui.QFormLayout(self.toolWidget)
        form.addRow("方块", self.blockButton)
        form.addRow(self.status)
        form.addRow(self.resetButton)

    def reset(self):
        self.previous = None
        self.count = 0
        self.cursorNode.visible = False
        self.overlayNode.visible = False
        self.status.setText("尚未选择起点")

    def toolActive(self):
        print("连续折线工具已激活：", self.editorSession.currentDimension.dimName)

    def mouseMove(self, event):
        self.cursorNode.visible = self.showCursorAction.isChecked() and event.blockPosition is not None
        if event.blockPosition is not None:
            self.cursorPosition.translateOffset = tuple(event.blockPosition)

    def cursorVisibility(self, visible):
        if not visible:
            self.cursorNode.visible = False

    def mousePress(self, event):
        position = tuple(event.blockPosition)
        start = position if self.previous is None else self.previous
        self.cursorPosition.translateOffset = position
        self.cursorNode.visible = self.showCursorAction.isChecked()
        self.previousPosition.translateOffset = start
        self.overlayNode.visible = self.previous is not None
        with self.editorSession.beginSimpleCommand("连续折线"):
            sections = {}
            for x, y, z in bresenham(start, position):
                sections.setdefault((x >> 4, y >> 4, z >> 4), []).append((x, y, z))
            for points in sections.values():
                x, y, z = zip(*points)
                self.dimension.setBlocks(x, y, z, Blocks=self.blockButton.block)
        self.previous = position
        self.count += 1
        self.status.setText("已选择 %d 个端点；当前端点 %s" % (self.count, position))

    def toolInactive(self):
        print("连续折线工具已停用，本次选择端点数：", self.count)
        self.reset()
