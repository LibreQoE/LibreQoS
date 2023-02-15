import os

def checkPythonVersion():
    # Perform a version check
    import sys
    print("Running Python Version " + sys.version)
    version_ok = True
    if sys.version_info[0] < 3:
        version_ok = False
    if sys.version_info[0]==3 and sys.version_info[1] < 10:
        version_ok = False
    if version_ok == False:
        print("LibreQoS requires Python 3.10 or greater.")
        os._exit(-1)