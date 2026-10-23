> **MCEdit2 Next experimental Python 3 branch**
>
> You are viewing `python3-modern-editor`, an in-progress Qt6 and native Java
> Anvil port. The legacy Python 2.7 instructions further down refer to the
> original MCEdit2 and are **not** suitable for this branch's new application.
> Use Python 3.12+, `python -m pip install -e .`, then
> `python -m mcedit2_next`. The new app has **2D slice and experimental 3D OpenGL voxel views** (optional client-JAR
> full-cube textures),
> a plugin manager, undoable block editing, Java Anvil (.mca) and old MCRegion (.mcr)
> original-format editing, CloneTool-style original-state repeat/tiling,
> with opt-in **ignore source air** across offset, repeated, rotated,
> mirrored, scaled, rotate-then-scale and reduced copies; the 3D ghost
> preview uses the same original-air mask and leaves target cells
> beneath skipped source air unchanged. A separate opt-in **paste only
> into exact destination air** mode is now available for all six
> block-only clone paths, retaining occupied native blocks and BE NBT.
> In this mode the 3D preview uses wireframes only; Apply checks the
> original destination occupancy and all accepted write targets again.
> Every source Block Entity remains protected,
> strict 90°/180°/270° horizontal rotation or X/Z-mirrored copies,
> plus guarded **Up/Down (Y) mirror** for verified vanilla stairs,
> slabs, trapdoors and limited original numeric metadata (all unsupported
> orientation/link semantics remain refused), and nearest-neighbour
> integer XYZ scaling (1–8× per axis), including **rotation followed by
> per-output-axis scaling** with native BE-protected placement, plus **integer XYZ
> downscaling** (independent divisors 1–8; nearest-neighbour sampling
> with trailing partial groups retained). Reduction checks every
> source coordinate for a protected Block Entity, even unsampled
> blocks, and preflights all native targets before any write.
> The Clone Tool now shows per-copy cyan/green 3D target-volume
> wireframes in both OpenGL view paths: preview, orbit/zoom,
> Ctrl+Alt+left-drag the white center control in X/Z, or
> Ctrl+Alt+Shift+left-drag vertically, with 1/2/4/8/16-block absolute
> world-coordinate snapping. The dock separately maintains
> **Step XYZ** (copy spacing) and **Group position XYZ** (rigid group
> placement). Dragging moves all copies together without altering
> their n×step spacing. Apply revalidates original native data,
> while Clear dismisses the preview without writes.
> For previews of up to **2,048 planned block cells**, both 3D
> renderers also display a read-only semi-transparent **full-cube
> material/color ghost** from one cached original BlockState snapshot.
> The streamed renderer uses the existing texture atlas without
> remeshing native world chunks; custom multi-element models and
> fluid surfaces are not yet accurate per-block ghosts. Larger or
> unsafe selections fall back to the wireframe, with a UI explanation.
> This is not proof that target BEs/sections are writable. All block-only
> cloning operations share single-step Undo, full BE/native target
> preflight and persistent Qt6 copy-count/orientation/scale controls;
> guarded BE NBT scalar/structure
> editing remains separate,
> a level-aware 3D fluid preview, guarded world/player metadata
> scalar editing, and restricted original Java player **ItemStack count,
> slot moves/swaps, Inventory↔EnderItems transfers, split, strict merge and
> explicit deletion**. Unknown modded item NBT is preserved through original
> byte-span edits; unsupported custom slots and unequal mod data are rejected.
> The editor cannot infer each item's registry maximum stack size.
> The Qt6 **Filled Maps** panel now supports a staged pixel painting canvas,
> raw map palette byte editing, limited original map scalar metadata and
> independent map_N.dat gzip backup/restore. Unsupported modern colors
> have diagnostic previews, not invented palette mappings. Qt6 entity
> inspection additionally supports undoable original Pos, yaw/pitch
> Rotation, preflighted same-chunk multi-entity movement, and restricted
> existing-typed entity scalar flags/Health/AbsorptionAmount edits.
> It also supports guarded multi-selection deletion and same-chunk
> UUID-fresh duplication of original standalone dropped items/XP orbs,
> preserving existing modded ItemStack/unknown NBT bytes. Native
> original-UUID transfer between already-existing Java chunks is also
> available for standalone dropped items/XP orbs, even across region
> files, with source/target full-region backups. Multiple original items
> and XP orbs may transfer together by XYZ offset as one preflighted,
> Undo/Redo-grouped action; any unsafe target refuses the whole group.
> Missing or unrecognized
> UUID layouts, referenced/linked/modded entities and cross-dimension
> transfers remain refused. Entity List changes support Undo/Redo
> **before saving**; after saving, recovery uses the complete native
> region backup because original List indices can shift.
> Complete original-file backups and explicit restoration are provided. The **experimental BE-aware clipboard**
> remains opt-in (Java 1.13+ only).
> Metadata, ItemStack and map edits commit directly to their original
> files, separate from region Undo/Redo; nondestructive native entity
> edits use the region EditSession Undo/Redo. Version 0.3.0a58 has
> not passed the full CI/game-world validation matrix; Windows packaging remains disabled.
> It is **not yet fully equivalent to the original MCEdit2 editor**. Do not edit live or valuable maps. See the [migration checklist](MIGRATION_STATUS.md) for a feature-by-feature audit. For the
> compatibility limits, developer instructions and plugin API, see
> [MODERNIZATION.md](MODERNIZATION.md) and
> [plugin development](doc/modern_plugins.md).

[![Build status](https://ci.appveyor.com/api/projects/status/76956gfy2n7sl5me?svg=true)](https://ci.appveyor.com/project/codewarrior0/mcedit2)
[![Build Status](https://travis-ci.org/mcedit/mcedit2.svg?branch=master)](https://travis-ci.org/mcedit/mcedit2)

# MCEdit 2.0

MCEdit 2.0 is the next version of MCEdit, the World Editor for Minecraft. MCEdit allows you to edit every aspect of
a Minecraft world, and to import and export .schematic files created by many programs including WorldEdit and the
original MCEdit 1.x. It is free to use and licensed under the BSD license.

MCEdit 2.0's development is in the alpha stage. Many features may be buggy or missing. To download MCEdit 2.0 anyway, head over to
http://www.mcedit.net/

The rest of this file is of interest to programmers only.

# Getting Started

This guide is written with Windows developers in mind. Linux / OS X developers can read how to install below. 
Windows developers are assumed to be using a unix shell such as the _GIT Bash_ included with the Windows distribution 
of Git.

- Install [Python for Windows v2.7.9](http://www.python.org/downloads/). Edit your PATH environment variable (or your 
.bashrc) to have both the `python27` and `python27\scripts` folders. (by default, `c:\python27;c:\python27\scripts;`)
- Install [Microsoft Visual C++ Compiler for Python 2.7](http://www.microsoft.com/en-us/download/details.aspx?id=44266). 
This is not needed if you already have MSVC 2008 (Visual Studio 9.0) or the Windows SDK 7.0 
installed, but you probably don't so install it anyway.
- Install virtualenv: `pip install virtualenv` (pip is now included with recent versions of Python)
- Update to the latest setuptools to make sure Cython modules compile: `pip install -U setuptools`
- Create a local clone of the MCEdit sources: `git clone https://github.com/mcedit/mcedit2`
- Using the `bash` shell, change directory to the local clone: `cd mcedit2`
- Create a virtualenv using `virtualenv ENV`
- Activate the virtualenv using `. ENV/scripts/activate`

A virtualenv is created in the local clone directory to keep MCEdit's required libraries isolated from those
installed in the systemwide Python folder. This prevents unpleasant surprises when you update a library 
for another project and later find it isn't compatible with MCEdit.

Now, install the required libraries. 

On Windows, `easy_install` is able to install binary installer packages into a virtualenv. Download each of the following and run `easy_install <module>.exe` while the virtualenv is active. Binary packages for the following are available at Chris Gohlke's page. Download the versions for Python 2.7, and make sure to download the `win32` versions if you installed the 32-bit Python (the default) or the `win-amd64` versions for 64-bit Python.

- [pyside](http://www.lfd.uci.edu/~gohlke/pythonlibs#pyside) 
- [pyopengl](http://www.lfd.uci.edu/~gohlke/pythonlibs#pyopengl) (be sure to grab `PyOpenGL-accelerate` too.)
- [pywin32](http://www.lfd.uci.edu/~gohlke/pythonlibs#pywin32) (for registry access)
- [cython](http://www.lfd.uci.edu/~gohlke/pythonlibs#cython) (for building `nbt.pyd`)
- [ipython](http://www.lfd.uci.edu/~gohlke/pythonlibs#ipython) (for debugging)
- [pygments](http://www.lfd.uci.edu/~gohlke/pythonlibs#pygments) (required by IPython) 
- [numpy](http://www.lfd.uci.edu/~gohlke/pythonlibs#numpy) **

Also use _pip_ to install these libraries:

- `pip install arrow` - a date/time class with nice text formatting.
- `pip install pyzmq` - Gohlke's pyzmq builds don't work on Windows XP

**Note that Gohlke's numpy builds use the Intel Math Kernel Library (MKL) which requires a license to use. If you 
have not purchased an MKL license (it's expensive) then you ***DO NOT HAVE PERMISSION TO DISTRIBUTE APPS*** built 
with it. I didn't notice any better performance with MKL regardless.

An alternative is to install the official builds of numpy from the [SourceForge Downloads](http://sourceforge.net/projects/numpy/files/NumPy/)
but 64-bit builds are not provided. If you need a 64-bit build of numpy that does not include MKL, you will need to 
build it yourself. Also, the official builds are packed into a "superpack" installer which easy_install chokes on. 
Just open the installer in an archiving program like [7-zip](http://www.7-zip.org/) and extract the SSE3 installer, 
then `easy_install` it.

Another alternative to downloading all of the above is to download all the packages 
[from my dropbox folder](https://www.dropbox.com/sh/fw8u5f050r1m6lp/AABAYXOEAzmV_nfH0Qj9hUOwa?dl=0). Note that the 
64-bit numpy package is one I built myself, with several facilities (LAPACK, BLAS etc) disabled because I couldn't 
be bothered to find out why MSVC 64-bit chokes on them. MCEdit seems to run fine without them. Install 
`python-2.7.9.msi` first, then install [MSVC++ for Python](http://www.microsoft.com/en-us/download/details.aspx?id=44266), 
then virtualenv, and create and activate a virtualenv as above. Install each of the .exe files using `easy_install` 
and make sure to `pip install arrow` too.

Once all of the requirements are met, install MCEdit itself into the virtualenv. This will build `nbt.pyd`, ensure 
both `mcedit2` and `mceditlib` are on the pythonpath, and also create an `mcedit2` script making it easy to launch 
the app. 

`python setup.py develop`

All that's left is to see if the app launches.

`mcedit2`

As a bonus, you can use the `-debug` flag to enable the Debug menu and a few extra widgets.

`mcedit2 -debug`

## Linux/OS X (not tested fully)

    cd (Your mcedit2 location)
    virtualenv ENV
    . ENV/bin/activate
    pip install -r requirements.txt
    python setup.py develop
    mcedit2

If your distro packages python3 as the default version of python instead of python2, you will probably want to set your virtualenv to use python2, i.e.

    virtualenv -p python2 ENV

If you get a `Library not loaded: libpyside-python2.7.1.2.dylib` error, try running `pyside_postinstall.py -install` to fix this. 

If after that, you get a `Library not loaded: /usr/local/lib/QtGui.framework/Versions/4/QtGui` error after that, try installing `qt` using a package manager like Homebrew or apt-get or similar equivalent on your distribution.   You may also need `libxslt-dev` and `python-dev` packages installed before the commands above will work.

Example: `brew install qt` etc. or:

    sudo apt-get install qt-sdk
    sudo apt-get install libxslt-dev
    sudo apt-get install python-dev

_Note: This information is not fully tested and might not work for you_

# Troubleshooting

- `python setup.py develop` or `build` produces the error `cannot find vcvarsall.bat` or similar.
Old version of setuptools don't know about MSVC++ for Python 2.7. Run `pip install --upgrade setuptools` to upgrade.

- When running mcedit2, `ImportError: cannot import name nbt` is produced.
The extension module `nbt.pyd` failed to build. Run `python setup.py develop` to rebuild it and look for any errors.
 The most likely error is the `vcvarsall.bat` error above.
 
- When running mcedit2, `ImportError: %1 is not a valid Win32 application` is produced.
This happens when switching between 32-bit and 64-bit Pythons on Windows. The `nbt.pyx` must be rebuilt after 
switching, so run `python setup.py develop` again.
