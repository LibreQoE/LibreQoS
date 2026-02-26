# Comprehensively handling errors from the OS is a difficult job
# In test, they rarely happen. In production, doing sane things
# IN ALL CASES, is sane. 

# Deciding what the sane things are, for each error return, is hard.

# The example pinned.c file is a good example - What goes wrong if
# ANY of these operations fail? What are the symptoms? Merely
# removing the file does not mean that the daemon holding it open
# notices

See:

https://github.com/LibreQoE/LibreQoS/issues/209
https://github.com/LibreQoE/LibreQoS/issues/208
https://github.com/LibreQoE/LibreQoS/issues/118   

And several closed bugs, where EEXIST was not an error. Etc.

