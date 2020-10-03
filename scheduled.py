# Copyright (C) 2020  Robert Chacón
# This file is part of LibreQoS.
#
# LibreQoS is free software: you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation, either version 2 of the License, or
# (at your option) any later version.
#
# LibreQoS is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with LibreQoS.  If not, see <http://www.gnu.org/licenses/>.
#
#            _     _ _               ___       ____  
#           | |   (_) |__  _ __ ___ / _ \  ___/ ___| 
#           | |   | | '_ \| '__/ _ \ | | |/ _ \___ \ 
#           | |___| | |_) | | |  __/ |_| | (_) |__) |
#           |_____|_|_.__/|_|  \___|\__\_\\___/____/
#                           v.0.4-alpha
#
import time
import schedule
from datetime import date
from LibreQoS import refreshShapers

if __name__ == '__main__':
	refreshShapers()
	schedule.every().day.at("04:00").do(refreshShapers)
	while True:
		schedule.run_pending()
		time.sleep(60) # wait one minute
