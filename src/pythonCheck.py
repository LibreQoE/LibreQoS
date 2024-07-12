import os

def checkPythonVersion():
    # Perform a version check
    import sys
    print("Running Python Version " + sys.version)
    if sys.version_info[0] < 3 or (sys.version_info[0]==3 and sys.version_info[1] < 10):
        print("LibreQoS requires Python 3.10 or greater.")
        os._exit(-1)
