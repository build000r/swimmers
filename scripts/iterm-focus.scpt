on sessionPaneId(aSession)
	try
		tell application id "com.googlecode.iterm2"
			return (unique id of aSession) as text
		end tell
	on error
		return ""
	end try
end sessionPaneId

on sessionTag(aSession)
	try
		tell application id "com.googlecode.iterm2"
			tell aSession to set taggedId to variable "user.throngterm.session_id"
		end tell
		return taggedId as text
	on error
		return ""
	end try
end sessionTag

on focusExistingSession(aWindow, aTab, aSession, targetSessionId, displayName)
	my markWorkspaceSession(aSession, targetSessionId, displayName)
	tell application id "com.googlecode.iterm2"
		tell aWindow to select
		tell aTab to select
		tell aSession to select
		activate
	end tell
	return my encodeResult("focused", aSession)
end focusExistingSession

on findSessionByPaneId(targetPaneId, targetSessionId, displayName)
	if targetPaneId is "" then return ""
	tell application id "com.googlecode.iterm2"
		repeat with aWindow in windows
			repeat with aTab in tabs of aWindow
				repeat with aSession in sessions of aTab
					if (my sessionPaneId(aSession)) is targetPaneId then
						return my focusExistingSession(aWindow, aTab, aSession, targetSessionId, displayName)
					end if
				end repeat
			end repeat
		end repeat
	end tell
	return ""
end findSessionByPaneId

on findSessionByTag(targetSessionId, displayName)
	tell application id "com.googlecode.iterm2"
		repeat with aWindow in windows
			repeat with aTab in tabs of aWindow
				repeat with aSession in sessions of aTab
					if (my sessionTag(aSession)) is targetSessionId then
						return my focusExistingSession(aWindow, aTab, aSession, targetSessionId, displayName)
					end if
				end repeat
			end repeat
		end repeat
	end tell
	return ""
end findSessionByTag

on encodeResult(statusText, aSession)
	tell application id "com.googlecode.iterm2"
		set paneId to unique id of aSession
	end tell
	return statusText & "|" & paneId
end encodeResult

on findWorkspaceTab()
	tell application id "com.googlecode.iterm2"
		repeat with aWindow in windows
			repeat with aTab in tabs of aWindow
				repeat with aSession in sessions of aTab
					try
						tell aSession to set workspaceId to variable "user.throngterm.workspace"
						if workspaceId is "main" then
							return aTab
						end if
					end try
				end repeat
			end repeat
		end repeat
	end tell
	return missing value
end findWorkspaceTab

on chooseSplitTarget(targetTab)
	set bestSession to missing value
	set bestArea to 0
	tell application id "com.googlecode.iterm2"
		repeat with aSession in sessions of targetTab
			try
				set currentCols to columns of aSession
				set currentRows to rows of aSession
				set currentArea to currentCols * currentRows
				if bestSession is missing value or currentArea > bestArea then
					set bestSession to aSession
					set bestArea to currentArea
				end if
			end try
		end repeat
	end tell
	return bestSession
end chooseSplitTarget

on markWorkspaceSession(aSession, targetSessionId, displayName)
	set resolvedName to displayName
	if resolvedName is "" then set resolvedName to "Throngterm"
	tell application id "com.googlecode.iterm2"
		tell aSession
			set variable named "user.throngterm.workspace" to "main"
			set variable named "user.throngterm.session_id" to targetSessionId
			try
				set name to resolvedName
			end try
		end tell
	end tell
end markWorkspaceSession

on preferredWindow()
	tell application id "com.googlecode.iterm2"
		if (count of windows) is 0 then return missing value
		try
			return current window
		on error
			return first window
		end try
	end tell
end preferredWindow

on tabFromWindow(aWindow)
	if aWindow is missing value then return missing value
	tell application id "com.googlecode.iterm2"
		repeat with attemptIndex from 1 to 5
			try
				return current tab of aWindow
			end try
			try
				return first tab of aWindow
			end try
			if attemptIndex is less than 5 then delay 0.1
		end repeat
	end tell
	return missing value
end tabFromWindow

on sessionFromTab(aTab)
	if aTab is missing value then return missing value
	tell application id "com.googlecode.iterm2"
		repeat with attemptIndex from 1 to 5
			try
				return current session of aTab
			end try
			try
				return first session of aTab
			end try
			if attemptIndex is less than 5 then delay 0.1
		end repeat
	end tell
	return missing value
end sessionFromTab

on createWorkspaceTab(attachCommand, targetSessionId, displayName)
	set newWindow to missing value
	set newTab to missing value
	tell application id "com.googlecode.iterm2"
		activate
		if (count of windows) is 0 then
			set newWindow to (create window with default profile command attachCommand)
		else
			set targetWindow to my preferredWindow()
			if targetWindow is missing value then
				set newWindow to (create window with default profile command attachCommand)
			else
				try
					tell targetWindow
						set newTab to (create tab with default profile command attachCommand)
					end tell
				on error
					set newWindow to (create window with default profile command attachCommand)
				end try
			end if
		end if
	end tell

	if newTab is missing value then
		if newWindow is not missing value then
			set newTab to my tabFromWindow(newWindow)
		else
			set targetWindow to my preferredWindow()
			if targetWindow is not missing value then set newTab to my tabFromWindow(targetWindow)
		end if
	end if

	set newSession to my sessionFromTab(newTab)
	if newSession is missing value then error "unable to resolve iTerm session after tab creation"
	my markWorkspaceSession(newSession, targetSessionId, displayName)
	return newSession
end createWorkspaceTab

on createOrSplitSession(targetSessionId, tmuxName, attachCommand, displayName)
	set workspaceTab to my findWorkspaceTab()
	if workspaceTab is missing value then
		set createdSession to my createWorkspaceTab(attachCommand, targetSessionId, displayName)
		return my encodeResult("created", createdSession)
	end if
	
	set splitSource to my chooseSplitTarget(workspaceTab)
	if splitSource is missing value then
		set createdSession to my createWorkspaceTab(attachCommand, targetSessionId, displayName)
		return my encodeResult("created", createdSession)
	end if
	
	tell application id "com.googlecode.iterm2"
		set sourceCols to columns of splitSource
		set sourceRows to rows of splitSource
	end tell
	
	set canSplitVertically to ((sourceCols / 2) is greater than or equal to 90)
	set canSplitHorizontally to ((sourceRows / 2) is greater than or equal to 18)
	
	if not canSplitVertically and not canSplitHorizontally then
		set createdSession to my createWorkspaceTab(attachCommand, targetSessionId, displayName)
		return my encodeResult("created", createdSession)
	end if
	
	set newSession to missing value
	tell application id "com.googlecode.iterm2"
		tell splitSource
			try
				if canSplitVertically and (sourceCols is greater than or equal to sourceRows or not canSplitHorizontally) then
					set newSession to split vertically with default profile command attachCommand
				else
					set newSession to split horizontally with default profile command attachCommand
				end if
			end try
		end tell
	end tell

	if newSession is missing value then
		set createdSession to my createWorkspaceTab(attachCommand, targetSessionId, displayName)
		return my encodeResult("created", createdSession)
	end if
	
	my markWorkspaceSession(newSession, targetSessionId, displayName)
	tell application id "com.googlecode.iterm2"
		tell newSession to select
		activate
	end tell
	return my encodeResult("created", newSession)
end createOrSplitSession

on run argv
	if (count of argv) is less than 4 then error "expected session_id, tmux_name, attach_command, and display_name"
	set targetSessionId to item 1 of argv
	set tmuxName to item 2 of argv
	set attachCommand to item 3 of argv
	set displayName to item 4 of argv
	set knownPaneId to ""
	if (count of argv) is greater than or equal to 5 then set knownPaneId to item 5 of argv
	if knownPaneId is not "" then
		set existingSession to my findSessionByPaneId(knownPaneId, targetSessionId, displayName)
		if existingSession is not "" then
			return existingSession
		end if
	end if
	repeat with attemptIndex from 1 to 3
		set existingSession to my findSessionByTag(targetSessionId, displayName)
		if existingSession is not "" then
			return existingSession
		end if
		if attemptIndex is less than 3 then delay 0.1
	end repeat
	return my createOrSplitSession(targetSessionId, tmuxName, attachCommand, displayName)
end run
