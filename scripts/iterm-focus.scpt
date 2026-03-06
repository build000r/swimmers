on findSessionByTag(targetSessionId)
	tell application id "com.googlecode.iterm2"
		repeat with aWindow in windows
			repeat with aTab in tabs of aWindow
				repeat with aSession in sessions of aTab
					try
						tell aSession to set taggedId to variable "user.throngterm.session_id"
						if taggedId is targetSessionId then
							tell aWindow to select
							tell aTab to select
							tell aSession to select
							activate
							return my encodeResult("focused", aSession)
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

on markWorkspaceSession(aSession, targetSessionId)
	tell application id "com.googlecode.iterm2"
		tell aSession
			set variable named "user.throngterm.workspace" to "main"
			set variable named "user.throngterm.session_id" to targetSessionId
			try
				set name to "Throngterm"
			end try
		end tell
	end tell
end markWorkspaceSession

on createWorkspaceTab(attachCommand, targetSessionId)
	tell application id "com.googlecode.iterm2"
		activate
		if (count of windows) is 0 then
			create window with default profile command attachCommand
			set newSession to current session of current tab of current window
			my markWorkspaceSession(newSession, targetSessionId)
			return newSession
		end if
		
		tell current window
			create tab with default profile command attachCommand
			set newSession to current session of current tab
		end tell
		my markWorkspaceSession(newSession, targetSessionId)
		return newSession
	end tell
end createWorkspaceTab

on createOrSplitSession(targetSessionId, tmuxName, tmuxPath)
	set attachCommand to tmuxPath & " attach -t " & quoted form of tmuxName
	set workspaceTab to my findWorkspaceTab()
	if workspaceTab is missing value then
		set createdSession to my createWorkspaceTab(attachCommand, targetSessionId)
		return my encodeResult("created", createdSession)
	end if
	
	set splitSource to my chooseSplitTarget(workspaceTab)
	if splitSource is missing value then
		set createdSession to my createWorkspaceTab(attachCommand, targetSessionId)
		return my encodeResult("created", createdSession)
	end if
	
	tell application id "com.googlecode.iterm2"
		set sourceCols to columns of splitSource
		set sourceRows to rows of splitSource
	end tell
	
	set canSplitVertically to ((sourceCols / 2) is greater than or equal to 90)
	set canSplitHorizontally to ((sourceRows / 2) is greater than or equal to 18)
	
	if not canSplitVertically and not canSplitHorizontally then
		set createdSession to my createWorkspaceTab(attachCommand, targetSessionId)
		return my encodeResult("created", createdSession)
	end if
	
	tell application id "com.googlecode.iterm2"
		tell splitSource
			if canSplitVertically and (sourceCols is greater than or equal to sourceRows or not canSplitHorizontally) then
				set newSession to split vertically with default profile command attachCommand
			else
				set newSession to split horizontally with default profile command attachCommand
			end if
		end tell
	end tell
	
	my markWorkspaceSession(newSession, targetSessionId)
	tell application id "com.googlecode.iterm2"
		tell newSession to select
		activate
	end tell
	return my encodeResult("created", newSession)
end createOrSplitSession

on run argv
	if (count of argv) is less than 3 then error "expected session_id, tmux_name, and tmux_path"
	set targetSessionId to item 1 of argv
	set tmuxName to item 2 of argv
	set tmuxPath to item 3 of argv
	set existingSession to my findSessionByTag(targetSessionId)
	if existingSession is not "" then
		return existingSession
	end if
	return my createOrSplitSession(targetSessionId, tmuxName, tmuxPath)
end run
