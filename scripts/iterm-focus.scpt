on sessionPaneId(aSession)
	try
		tell application id "com.googlecode.iterm2"
			return (unique id of aSession) as text
		end tell
	on error
		return ""
	end try
end sessionPaneId

on focusExistingSession(aWindow, aTab, aSession, targetSessionId, displayName)
	set paneId to my sessionPaneId(aSession)
	my markWorkspaceSession(aSession, targetSessionId, displayName)
	tell application id "com.googlecode.iterm2"
		tell aWindow to select
		tell aTab to select
		tell aSession to select
		activate
	end tell
	return "focused|" & paneId
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
					try
						tell aSession to set taggedId to variable named "user.throngterm.session_id"
						if (taggedId as text) is targetSessionId then
							return my focusExistingSession(aWindow, aTab, aSession, targetSessionId, displayName)
						end if
					end try
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
						tell aSession to set workspaceId to variable named "user.throngterm.workspace"
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
		try
			tell aSession to set name to resolvedName
		end try
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

on lastWindowImmediate()
	tell application id "com.googlecode.iterm2"
		try
			return last window
		end try
	end tell
	return missing value
end lastWindowImmediate

on lastWindow()
	repeat with attemptIndex from 1 to 5
		set resolvedWindow to my lastWindowImmediate()
		if resolvedWindow is not missing value then return resolvedWindow
		if attemptIndex is less than 5 then delay 0.1
	end repeat
	return missing value
end lastWindow

on tabFromWindowImmediate(aWindow)
	if aWindow is missing value then return missing value
	tell application id "com.googlecode.iterm2"
		try
			return current tab of aWindow
		end try
		try
			return first tab of aWindow
		end try
	end tell
	return missing value
end tabFromWindowImmediate

on tabFromWindow(aWindow)
	if aWindow is missing value then return missing value
	repeat with attemptIndex from 1 to 5
		set resolvedTab to my tabFromWindowImmediate(aWindow)
		if resolvedTab is not missing value then return resolvedTab
		if attemptIndex is less than 5 then delay 0.1
	end repeat
	return missing value
end tabFromWindow

on lastTabFromWindowImmediate(aWindow)
	if aWindow is missing value then return missing value
	tell application id "com.googlecode.iterm2"
		try
			return last tab of aWindow
		end try
		try
			return current tab of aWindow
		end try
		try
			return first tab of aWindow
		end try
	end tell
	return missing value
end lastTabFromWindowImmediate

on lastTabFromWindow(aWindow)
	if aWindow is missing value then return missing value
	repeat with attemptIndex from 1 to 5
		set resolvedTab to my lastTabFromWindowImmediate(aWindow)
		if resolvedTab is not missing value then return resolvedTab
		if attemptIndex is less than 5 then delay 0.1
	end repeat
	return missing value
end lastTabFromWindow

on sessionFromTabImmediate(aTab)
	if aTab is missing value then return missing value
	tell application id "com.googlecode.iterm2"
		try
			return current session of aTab
		end try
		try
			return first session of aTab
		end try
	end tell
	return missing value
end sessionFromTabImmediate

on sessionFromTab(aTab)
	if aTab is missing value then return missing value
	repeat with attemptIndex from 1 to 5
		set resolvedSession to my sessionFromTabImmediate(aTab)
		if resolvedSession is not missing value then return resolvedSession
		if attemptIndex is less than 5 then delay 0.1
	end repeat
	return missing value
end sessionFromTab

on resolveCreatedSession(targetWindow, candidateTab)
	if candidateTab is not missing value then
		repeat with attemptIndex from 1 to 10
			set candidateSession to my sessionFromTabImmediate(candidateTab)
			if candidateSession is not missing value then return candidateSession
			if attemptIndex is less than 10 then delay 0.1
		end repeat
		return my sessionFromTab(candidateTab)
	end if
	
	repeat with attemptIndex from 1 to 10
		if targetWindow is not missing value then
			set candidateTab to my lastTabFromWindowImmediate(targetWindow)
		end if
		set candidateSession to my sessionFromTabImmediate(candidateTab)
		if candidateSession is not missing value then return candidateSession
		if attemptIndex is less than 10 then delay 0.1
	end repeat
	if targetWindow is not missing value then
		set refreshedTab to my lastTabFromWindow(targetWindow)
		return my sessionFromTab(refreshedTab)
	end if
	return missing value
end resolveCreatedSession

on createWorkspaceTab(attachCommand, targetSessionId, displayName)
	set newWindow to missing value
	tell application id "com.googlecode.iterm2"
		activate
		try
			set newWindow to (create window with default profile)
		end try
	end tell
	
	set targetWindow to newWindow
	if targetWindow is missing value then set targetWindow to my lastWindow()
	set newTab to my lastTabFromWindow(targetWindow)
	
	set newSession to my resolveCreatedSession(targetWindow, newTab)
	if newSession is missing value then error "unable to resolve iTerm session after tab creation"
	tell application id "com.googlecode.iterm2"
		tell newSession to write text attachCommand
	end tell
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
