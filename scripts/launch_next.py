"""Packaged launcher; avoids executing the Python 2 MCEdit2 entrypoint."""
import sys

from mcedit2_next import __version__


def main():
    if "--version" in sys.argv:
        if sys.stdout is not None:
            print("MCEdit2 Next", __version__)
        return 0
    if "--selftest" in sys.argv:
        from PySide6.QtWidgets import QApplication
        from mcedit2_next.gui import MainWindow
        app = QApplication([])
        window = MainWindow()
        assert window.view.session is None
        assert window.plugin_menu is not None
        window.close()
        return 0
    from mcedit2_next.gui import main as gui_main
    return gui_main()


if __name__ == "__main__":
    raise SystemExit(main())
