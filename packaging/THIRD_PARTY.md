# Bundled runtime

The standalone helper bundle includes Python and standard library components,
PyInstaller's bootloader, tomli, and required system libraries. The four helper
entry points share this single runtime.

- Python: Python Software Foundation License, https://docs.python.org/3/license.html
- PyInstaller bootloader: GPL 2.0 with the bootloader exception permitting bundled applications, https://pyinstaller.org/en/stable/license.html
- tomli: MIT License, https://github.com/hukkin/tomli/blob/master/LICENSE
- OpenSSL: Apache License 2.0, https://www.openssl.org/source/license.html
- SQLite: public domain, https://sqlite.org/copyright.html

The build machine's library license notices are included under licenses/ in the release archive.
