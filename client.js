// Production-grade frontend for real-time chat
// Features: proper error handling, accessibility, mobile support, disconnect recovery

class ChatApp {
    constructor() {
        this.ws = null;
        this.myAnimalName = null;
        this.myUserId = null;
        this.connected = false;
        this.reconnectAttempts = 0;
        this.maxReconnectAttempts = 10;
        this.maxMessages = 500;
        this.typingUsers = new Map();
        this.typingTimeout = null;
        this.isCurrentlyTyping = false;
        this.lastReadMessageId = null;
        this.audioContext = null;
        this.roomName = this.extractRoomName();
        this.shouldAutoScroll = true;
        this.lastScrollTop = 0;
        // True from connect until the server's ReconnectToken frame, which it
        // sends immediately after the last history message. While set, per
        // message scrolling is suppressed entirely and the scroll handler
        // ignores events, so history loads without the view moving.
        this.isLoadingHistory = true;
        this.cleanupIntervalId = null;
        this.scrollTimeoutId = null;
        this.autoScrollFrameId = null;
        this.programmaticScroll = false;
        this.historyLoadTimeoutId = null;
        
        // Room heartbeat tracking
        this.roomStartTime = Date.now();
        this.lastActivityTime = Date.now();
        this.lastMessageTime = 0;
        this.recentMessageCount = 0;
        this.heartbeatIntervalId = null;
        this.fadeWarningShown = false;
        
        // Heartbeat monitoring - detect server silence
        this.lastHeartbeatTime = Date.now();
        this.heartbeatTimeoutId = null;
        
        // Sound mute state (persisted in localStorage)
        this.isMuted = localStorage.getItem('chatMuted') === 'true';
        
        // Reply state
        this.replyingTo = null; // { messageId, authorName, text }
        
        // DOM elements
        this.chat = document.getElementById('chat');
        this.input = document.getElementById('messageInput');
        this.sendButton = document.getElementById('sendBtn');
        this.userCountNumEl = document.getElementById('userCountNum');
        this.statusChip = document.getElementById('statusChip');
        this.typingIndicator = document.getElementById('typingIndicator');
        this.typingText = document.getElementById('typingText');
        this.statusText = document.getElementById('statusText');
        this.charCount = document.getElementById('charCount');
        this.roomNameEl = document.getElementById('roomName');
        this.welcomeBanner = document.getElementById('welcomeBanner');
        this.createRoomBtn = document.getElementById('createRoomBtn');
        this.roomHeartbeat = document.getElementById('roomHeartbeat');
        this.roomLifespan = document.getElementById('roomLifespan');
        this.muteBtn = document.getElementById('muteBtn');
        this.replyPreview = document.getElementById('replyPreview');
        this.replyPreviewAuthor = document.getElementById('replyPreviewAuthor');
        this.replyPreviewText = document.getElementById('replyPreviewText');
        this.replyPreviewClose = document.getElementById('replyPreviewClose');
        
        this.init();
    }
    
    init() {
        this.setupEventListeners();
        this.initAudioContext();
        this.updateRoomName();
        this.connect();
        
        // Periodic cleanup of stale typing indicators (store ID for cleanup)
        this.cleanupIntervalId = setInterval(() => this.cleanupTypingIndicators(), 1000);
        
        // Start heartbeat monitor
        this.heartbeatIntervalId = setInterval(() => this.updateHeartbeat(), 1000);
        
        // Monitor scroll position to control auto-scroll
        this.chat.addEventListener('scroll', (e) => {
            this.handleChatScroll();
        });
        
        // Cleanup on page unload
        window.addEventListener('beforeunload', () => this.cleanup());
        window.addEventListener('pagehide', () => this.cleanup());
    }
    
    setupEventListeners() {
        this.input.addEventListener('keydown', (e) => this.handleInputKeydown(e));
        this.input.addEventListener('input', (e) => this.handleInput(e));
        this.sendButton.addEventListener('click', () => this.sendMessage());
        this.sendButton.addEventListener('touchend', (e) => {
            e.preventDefault();
            this.sendMessage();
        });
        
        // Explore room link
        document.getElementById('exploreLink').addEventListener('click', (e) => {
            e.preventDefault();
            const rooms = ['mellow-forest', 'midnight-owl', 'coffee-talks', 'tech-minds', 'random-thoughts', 'chill-zone', 'late-night', 'creative-corner'];
            const room = rooms[Math.floor(Math.random() * rooms.length)];
            window.location.href = `/${room}`;
        });
        
        // Create room button
        this.createRoomBtn.addEventListener('click', () => {
            this.showCreateRoomDialog();
        });
        
        // Share room button
        const shareRoomBtn = document.getElementById('shareRoomBtn');
        if (shareRoomBtn) {
            shareRoomBtn.addEventListener('click', () => {
                this.shareRoom();
            });
        }
        
        // Dismiss banner
        const dismissBtn = document.getElementById('dismissBanner');
        if (dismissBtn) {
            dismissBtn.addEventListener('click', () => {
                this.welcomeBanner.style.display = 'none';
                localStorage.setItem('bannerDismissed', 'true');
            });
        }
        
        // Check if banner was previously dismissed
        if (localStorage.getItem('bannerDismissed') === 'true') {
            this.welcomeBanner.style.display = 'none';
        }
        
        // Mute button
        this.muteBtn.addEventListener('click', () => this.toggleMute());
        this.updateMuteButton();
        
        // Reply preview close
        this.replyPreviewClose.addEventListener('click', () => this.cancelReply());
        
        // Focus input on page load
        this.input.focus();
    }
    
    toggleMute() {
        this.isMuted = !this.isMuted;
        localStorage.setItem('chatMuted', this.isMuted);
        this.updateMuteButton();
        
        // Show feedback
        if (this.isMuted) {
            this.addSystemMessage('Notifications muted 🔇', 'info');
        } else {
            this.addSystemMessage('Notifications unmuted 🔊', 'info');
            // Play a test beep
            this.playNotificationSound(true);
        }
    }
    
    updateMuteButton() {
        // The icons are sprite references, not font ligatures: swap what the
        // <use> points at rather than the element's text.
        const use = this.muteBtn.querySelector('.icon use');
        const muted = this.isMuted;

        this.muteBtn.classList.toggle('muted', muted);
        this.muteBtn.title = muted ? 'Unmute notifications' : 'Mute notifications';
        this.muteBtn.setAttribute(
            'aria-label',
            muted ? 'Unmute notification sounds' : 'Mute notification sounds'
        );
        this.muteBtn.setAttribute('aria-pressed', String(muted));

        if (use) {
            use.setAttribute('href', muted ? '#i-volume-off' : '#i-volume-on');
        }
    }
    
    startReply(messageId, authorName, text) {
        // Strip HTML tags for preview
        const plainText = text.replace(/<[^>]*>/g, '').trim();
        this.replyingTo = { messageId, authorName, text: plainText };
        
        this.replyPreviewAuthor.textContent = `Replying to ${authorName}`;
        this.replyPreviewText.textContent = plainText.length > 100 ? plainText.slice(0, 100) + '...' : plainText;
        this.replyPreview.classList.add('active');
        this.input.focus();
    }
    
    cancelReply() {
        this.replyingTo = null;
        this.replyPreview.classList.remove('active');
    }
    
    scrollToMessage(messageId) {
        const targetMsg = this.chat.querySelector(`[data-message-id="${messageId}"]`);
        if (targetMsg) {
            targetMsg.scrollIntoView({ behavior: 'smooth', block: 'center' });
            targetMsg.classList.add('highlight');
            setTimeout(() => targetMsg.classList.remove('highlight'), 1500);
        }
    }
    
    showCreateRoomDialog() {
        const roomName = prompt('Enter a room name (3-50 characters):\\n\\nAllowed: letters, numbers, hyphens, underscores\\nExample: my-cool-room');
        if (roomName) {
            const trimmed = roomName.trim().toLowerCase();
            if (/^[a-zA-Z0-9][a-zA-Z0-9-_]{1,48}[a-zA-Z0-9]$/.test(trimmed)) {
                window.location.href = `/${trimmed}`;
            } else {
                this.showError('Invalid room name. Use 3-50 characters: letters, numbers, hyphens, underscores.');
            }
        }
    }
    
    shareRoom() {
        const url = window.location.href;
        if (navigator.share) {
            navigator.share({
                title: `Join me in #${this.roomName}`,
                text: 'Join this ephemeral chat room',
                url: url
            }).catch(() => {});
        } else if (navigator.clipboard) {
            navigator.clipboard.writeText(url).then(() => {
                this.addSystemMessage('Room link copied to clipboard!', 'success');
            }).catch(() => {
                this.addSystemMessage('Could not copy link', 'error');
            });
        }
    }
    
    initAudioContext() {
        try {
            const AudioContext = window.AudioContext || window.webkitAudioContext;
            this.audioContext = new AudioContext();
        } catch (e) {
            console.warn('AudioContext not available:', e);
        }
    }
    
    extractRoomName() {
        const pathParts = window.location.pathname.split('/').filter(p => p);
        return pathParts[pathParts.length - 1] || 'main';
    }
    
    updateRoomName() {
        this.roomNameEl.textContent = this.roomName;
    }
    
    connect() {
        const protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
        const wsURL = `${protocol}//${location.host}/ws/${this.roomName}`;

        // Every connection replays history, reconnections included, so the
        // suppression window reopens for each one.
        this.isLoadingHistory = true;

        // Failsafe: if ReconnectToken never arrives, auto-scroll would stay
        // suppressed for the whole session. Never let a missing frame leave
        // the UI permanently degraded.
        if (this.historyLoadTimeoutId) clearTimeout(this.historyLoadTimeoutId);
        this.historyLoadTimeoutId = setTimeout(() => this.finishHistoryLoad(), 5000);
        
        try {
            this.ws = new WebSocket(wsURL);
            
            this.ws.onopen = () => this.handleWebSocketOpen();
            this.ws.onclose = () => this.handleWebSocketClose();
            this.ws.onerror = (e) => this.handleWebSocketError(e);
            this.ws.onmessage = (e) => this.handleWebSocketMessage(e);
        } catch (e) {
            this.showError(`Failed to establish connection: ${e.message}`);
            this.updateConnectionStatus('disconnected');
        }
    }
    
    handleWebSocketOpen() {
        console.log('WebSocket connected');
        this.connected = true;
        // Reset activity tracking on connect
        this.lastActivityTime = Date.now();
        this.roomStartTime = Date.now();
        this.fadeWarningShown = false;
        if (this.reconnectAttempts > 0) {
            this.addSystemMessage('Reconnected successfully', 'success');
        }
        this.reconnectAttempts = 0;
        this.updateConnectionStatus('connected');
        this.sendButton.disabled = false;
        this.input.disabled = false;
        this.input.focus();
    }
    
    handleWebSocketClose() {
        console.log('WebSocket disconnected');
        if (this.heartbeatTimeoutId) clearTimeout(this.heartbeatTimeoutId);
        this.connected = false;
        this.updateConnectionStatus('disconnected');
        this.sendButton.disabled = true;
        this.input.disabled = true;
        this.tryReconnect();
    }
    
    handleWebSocketError(error) {
        console.error('WebSocket error:', error);
    }
    
    handleWebSocketMessage(event) {
        try {
            
            // Reset heartbeat timeout on any message (heartbeat + data)
            this.lastHeartbeatTime = Date.now();
            if (this.heartbeatTimeoutId) clearTimeout(this.heartbeatTimeoutId);
            
            // Set timeout for next heartbeat (server sends every 5s, timeout at 8s)
            this.heartbeatTimeoutId = setTimeout(() => {
                console.warn('Heartbeat timeout - server not responding');
                this.connected = false;
                this.updateConnectionStatus('disconnected');
                if (this.ws && this.ws.readyState === WebSocket.OPEN) {
                    this.ws.close();
                }
            }, 8000);
            
            const data = JSON.parse(event.data);
            this.handleServerEvent(data);
        } catch (e) {
            console.error('Failed to parse message:', e);
            this.showError('Received malformed message from server');
        }
    }
    
    tryReconnect() {
        if (this.reconnectAttempts >= this.maxReconnectAttempts) {
            this.showError('Unable to connect. Please refresh the page.');
            return;
        }
        
        const delay = Math.min(1000 * Math.pow(2, this.reconnectAttempts), 30000);
        this.updateConnectionStatus('connecting');
        
        // Update status text with countdown
        let remaining = Math.ceil(delay / 1000);
        this.statusText.textContent = `Reconnecting in ${remaining}s`;
        
        const countdownInterval = setInterval(() => {
            remaining--;
            if (remaining > 0 && !this.connected) {
                this.statusText.textContent = `Reconnecting in ${remaining}s`;
            } else {
                clearInterval(countdownInterval);
            }
        }, 1000);
        
        setTimeout(() => {
            clearInterval(countdownInterval);
            if (!this.connected && (!this.ws || this.ws.readyState !== WebSocket.CONNECTING)) {
                this.reconnectAttempts++;
                this.statusText.textContent = 'Connecting...';
                this.connect();
            }
        }, delay);
    }
    
    handleServerEvent(data) {
        switch (data.type) {
            case 'Welcome':
                // First frame on every connection: who we are. Everything that
                // distinguishes "my message" from "someone else's" depends on
                // this arriving before history, which the server guarantees.
                this.myUserId = data.user_id;
                this.myAnimalName = data.animal_name;
                break;
            case 'Message':
                this.handleIncomingMessage(data.message);
                break;
            case 'System':
                this.handleSystemEvent(data.event);
                break;
            case 'UserCount':
                this.updateUserCount(data.count);
                break;
            case 'Heartbeat':
                break;
            case 'ReconnectToken':
                // Marks the end of the history replay: the server sends this
                // straight after the last history message. One jump to the
                // bottom here replaces one animation per history frame.
                this.finishHistoryLoad();
                break;
            default:
                console.warn('Unknown event type:', data.type);
        }
    }
    
    handleIncomingMessage(msg) {
        if (!msg || !msg.message_id) {
            console.warn('Invalid message received:', msg);
            return;
        }
        
        // Track activity for heartbeat
        this.lastActivityTime = Date.now();
        this.fadeWarningShown = false;
        
        // Combo detection: rapid messages within 3 seconds
        const now = Date.now();
        const isCombo = (now - this.lastMessageTime) < 3000;
        this.lastMessageTime = now;
        if (isCombo) {
            this.recentMessageCount++;
        } else {
            this.recentMessageCount = 1;
        }
        
        // Identity comes from the Welcome frame, which the server sends before
        // any history. The cookies are HttpOnly, so document.cookie is empty by
        // design — never reintroduce a read of it here.
        const isSent = Boolean(this.myUserId) && msg.user_id === this.myUserId;

        this.renderMessage(msg, isSent, isCombo && this.recentMessageCount >= 2);

        // During history replay nothing scrolls; finishHistoryLoad does it once
        // at the end.
        if (!this.isLoadingHistory && this.shouldAutoScroll) {
            this.scrollToBottom();
        }

        // Play notification for received messages
        if (!isSent) {
            this.playNotificationSound();
            
            // Auto-send read receipt if at bottom
            if (this.isAtBottom()) {
                this.sendReadReceipt(msg.message_id);
            }
        }
    }
    
    renderMessage(msg, isSent, isCombo = false) {
        // Prevent duplicate message renders
        if (this.chat.querySelector(`[data-message-id="${msg.message_id}"]`)) {
            return;
        }
        
        const timestamp = this.formatTimestamp(this.parseTimestamp(msg.timestamp));
        
        const div = document.createElement('div');
        div.className = `message ${isSent ? 'sent' : 'received'}${isCombo ? ' combo' : ''}`;
        div.dataset.messageId = msg.message_id;
        div.dataset.userId = msg.user_id;
        div.setAttribute('role', 'article');
        div.setAttribute('aria-label', `Message from ${msg.animal_name} at ${timestamp}`);
        
        // Reply button
        const replyBtn = document.createElement('button');
        replyBtn.className = 'reply-btn';
        replyBtn.setAttribute('aria-label', 'Reply to this message');
        replyBtn.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-reply"/></svg>';
        replyBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            this.startReply(msg.message_id, msg.animal_name, msg.text);
        });
        div.appendChild(replyBtn);
        
        // If this message is a reply, show the replied-to content
        if (msg.reply_to) {
            const repliedTo = document.createElement('div');
            repliedTo.className = 'replied-to';
            repliedTo.innerHTML = `
                <div class="replied-to-icon"><svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-reply"/></svg></div>
                <div class="replied-to-content">
                    <div class="replied-to-author">${this.escapeHtml(msg.reply_to.author_name || 'Unknown')}</div>
                    <div class="replied-to-text">${this.escapeHtml(msg.reply_to.preview_text || '')}</div>
                </div>
            `;
            repliedTo.addEventListener('click', () => {
                this.scrollToMessage(msg.reply_to.message_id);
            });
            div.appendChild(repliedTo);
        }
        
        const avatar = document.createElement('div');
        avatar.className = 'avatar';
        avatar.setAttribute('aria-hidden', 'true');
        avatar.textContent = msg.animal_name[0].toUpperCase();
        
        const header = document.createElement('div');
        header.className = 'header';
        const name = document.createElement('strong');
        name.textContent = msg.animal_name;
        name.style.textOverflow = 'ellipsis';
        name.style.whiteSpace = 'nowrap';
        name.style.overflow = 'hidden';
        header.appendChild(avatar);
        header.appendChild(name);
        
        const content = document.createElement('div');
        content.className = 'message-content';
        content.innerHTML = msg.text; // Already sanitized by server
        
        const tsDiv = document.createElement('div');
        tsDiv.className = 'timestamp';
        tsDiv.textContent = timestamp;
        
        div.appendChild(header);
        div.appendChild(content);
        div.appendChild(tsDiv);
        
        this.chat.appendChild(div);
        
        this.pruneRenderedMessages();
    }

    /// Trims the oldest rendered messages once there are more than maxMessages.
    ///
    /// Counts the DOM directly rather than maintaining a running total. The
    /// counter this replaces was incremented for system messages too, but those
    /// remove themselves after 8 seconds without ever decrementing it — so on a
    /// busy room the count drifted well above the number of nodes actually
    /// present and began deleting live chat messages that were nowhere near the
    /// limit. Messages vanishing from a conversation is about the worst failure
    /// a chat client has, and it was invisible because the counter, not the
    /// page, was wrong.
    pruneRenderedMessages() {
        const messages = this.chat.querySelectorAll('.message');
        const excess = messages.length - this.maxMessages;

        for (let i = 0; i < excess; i++) {
            messages[i].remove();
        }
    }
    
    handleSystemEvent(event) {
        if (!event) return;
        
        if (event.UserJoined) {
            // Compare against the identity the server gave us rather than
            // assuming the first join we see is our own — with two people
            // connecting at once, that assumption picked the wrong user.
            if (event.UserJoined.user_id === this.myUserId) {
                this.addSystemMessage(`You joined as ${event.UserJoined.animal_name} 🎉`, 'success');
            } else {
                this.addSystemMessage(`${event.UserJoined.animal_name} joined`, 'success');
            }
        } else if (event.UserLeft) {
            this.typingUsers.delete(event.UserLeft.animal_name);
            this.updateTypingIndicator();
            this.addSystemMessage(`${event.UserLeft.animal_name} left`);
        } else if (event.Typing) {
            this.handleTypingIndicator(event.Typing.animal_name, event.Typing.is_typing);
        } else if (event.ReadReceipt) {
            this.updateReadReceipt(event.ReadReceipt.animal_name, event.ReadReceipt.message_id);
        } else if (event.ServerShutdown) {
            this.addSystemMessage(`Server is restarting: ${event.ServerShutdown.reason}`, 'warning');
        }
        
        if (!this.isLoadingHistory && this.shouldAutoScroll) {
            this.scrollToBottom();
        }
    }

    handleTypingIndicator(name, isTyping) {
        if (name === this.myAnimalName) return;
        
        if (isTyping) {
            this.typingUsers.set(name, Date.now());
        } else {
            this.typingUsers.delete(name);
        }
        this.updateTypingIndicator();
    }
    
    cleanupTypingIndicators() {
        const now = Date.now();
        let cleaned = false;
        
        for (const [name, timestamp] of this.typingUsers) {
            if (now - timestamp > 5000) {
                this.typingUsers.delete(name);
                cleaned = true;
            }
        }
        
        if (cleaned) {
            this.updateTypingIndicator();
        }
    }
    
    updateTypingIndicator() {
        const names = Array.from(this.typingUsers.keys());
        
        if (names.length === 0) {
            this.typingIndicator.style.display = 'none';
        } else {
            let text;
            if (names.length === 1) {
                text = `${names[0]} is typing...`;
            } else if (names.length === 2) {
                text = `${names[0]} and ${names[1]} are typing...`;
            } else {
                text = `${names.length} people are typing...`;
            }
            
            this.typingText.textContent = text;
            this.typingIndicator.style.display = 'flex';
        }
    }
    
    updateReadReceipt(animal_name, message_id) {
        if (animal_name === this.myAnimalName) {
            this.lastReadMessageId = message_id;
        }
    }
    
    handleInputKeydown(e) {
        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            this.sendMessage();
        }
    }
    
    handleInput(e) {
        const text = e.target.value;
        this.charCount.textContent = text.length;
        
        // Send typing indicator
        if (text.length > 0) {
            this.handleTyping();
        } else {
            this.cancelTyping();
        }
    }
    
    handleTyping() {
        if (!this.isCurrentlyTyping) {
            this.isCurrentlyTyping = true;
            this.sendTypingIndicator(true);
        }
        
        if (this.typingTimeout) clearTimeout(this.typingTimeout);
        this.typingTimeout = setTimeout(() => {
            this.cancelTyping();
        }, 3000);
    }
    
    cancelTyping() {
        if (this.isCurrentlyTyping) {
            this.isCurrentlyTyping = false;
            this.sendTypingIndicator(false);
        }
    }
    
    sendMessage() {
        const text = this.input.value.trim();
        
        if (!text) {
            this.input.focus();
            return;
        }
        
        if (!this.connected) {
            this.showError('Not connected. Attempting to reconnect...');
            return;
        }
        
        if (text.length > 8000) {
            this.showError('Message is too long (max 8000 characters)');
            return;
        }
        
        try {
            const messagePayload = { type: 'Message', text };
            
            // Include reply info if replying
            if (this.replyingTo) {
                messagePayload.reply_to = {
                    message_id: this.replyingTo.messageId,
                    author_name: this.replyingTo.authorName,
                    preview_text: this.replyingTo.text.slice(0, 100)
                };
            }
            
            this.ws.send(JSON.stringify(messagePayload));
            this.input.value = '';
            this.charCount.textContent = '0';
            this.isCurrentlyTyping = false;
            this.cancelReply(); // Clear reply state after sending
            if (this.typingTimeout) clearTimeout(this.typingTimeout);
            this.sendTypingIndicator(false);
            this.input.focus();
        } catch (e) {
            console.error('Failed to send message:', e);
            this.showError('Failed to send message. Please try again.');
        }
    }
    
    sendTypingIndicator(is_typing) {
        if (this.connected && this.ws.readyState === WebSocket.OPEN) {
            try {
                this.ws.send(JSON.stringify({ type: 'Typing', is_typing }));
            } catch (e) {
                console.warn('Failed to send typing indicator:', e);
            }
        }
    }
    
    sendReadReceipt(message_id) {
        if (this.connected && message_id && this.ws.readyState === WebSocket.OPEN) {
            try {
                this.ws.send(JSON.stringify({ type: 'ReadReceipt', message_id }));
            } catch (e) {
                console.warn('Failed to send read receipt:', e);
            }
        }
    }
    
    /// Pins the view to the newest message.
    ///
    /// Coalesced into one animation frame: a burst of messages must cost one
    /// scroll write after layout, not one per message. Scrolling from a
    /// `setTimeout(0)` ran before layout had settled, so the target height was
    /// occasionally stale and the view stopped a message short of the bottom.
    scrollToBottom() {
        if (this.autoScrollFrameId) return;

        this.autoScrollFrameId = requestAnimationFrame(() => {
            this.autoScrollFrameId = null;
            // Suppress the scroll handler for the event this write produces:
            // mid-scroll the element is not yet at the bottom, and reading that
            // back would clear shouldAutoScroll and strand the user.
            this.programmaticScroll = true;
            this.chat.scrollTop = this.chat.scrollHeight;
            requestAnimationFrame(() => { this.programmaticScroll = false; });
        });
    }

    /// Called on the ReconnectToken frame, which follows the last history
    /// message.
    finishHistoryLoad() {
        if (this.historyLoadTimeoutId) {
            clearTimeout(this.historyLoadTimeoutId);
            this.historyLoadTimeoutId = null;
        }
        if (!this.isLoadingHistory) return;

        this.isLoadingHistory = false;
        this.shouldAutoScroll = true;
        this.scrollToBottom();
    }

    handleChatScroll() {
        // Ignore scroll events the client caused itself, and everything during
        // history replay. Both would otherwise be read as "the user scrolled
        // up" and would switch auto-scroll off at exactly the wrong moment —
        // the intermittent part of the reload jumping.
        if (this.programmaticScroll || this.isLoadingHistory) return;

        const wasAtBottom = this.shouldAutoScroll;
        this.shouldAutoScroll = this.isAtBottom();

        // Only send read receipt if user just scrolled to bottom (debounce)
        if (this.shouldAutoScroll && !wasAtBottom) {
            const messages = this.chat.querySelectorAll('.message');
            if (messages.length > 0) {
                const lastMsg = messages[messages.length - 1];
                const messageID = lastMsg.dataset.messageId;
                if (messageID && messageID !== this.lastReadMessageId) {
                    this.sendReadReceipt(messageID);
                }
            }
        }
    }
    
    isAtBottom() {
        return this.chat.scrollHeight - this.chat.scrollTop - this.chat.clientHeight < 50;
    }
    
    updateUserCount(count) {
        this.userCountNumEl.textContent = count;
    }
    
    updateConnectionStatus(status) {
        this.statusChip.className = `status-chip ${status}`;
        
        switch (status) {
            case 'connected':
                this.statusText.textContent = 'Connected';
                break;
            case 'disconnected':
                this.statusText.textContent = 'Disconnected';
                break;
            case 'connecting':
                this.statusText.textContent = 'Connecting...';
                break;
        }
    }
    
    addSystemMessage(text, type = 'info') {
        const div = document.createElement('div');
        div.className = `system-message ${type}`;
        div.setAttribute('role', 'status');
        div.textContent = text;
        this.chat.appendChild(div);
        
        // Auto-remove after 8 seconds
        setTimeout(() => {
            if (div.parentNode) {
                div.remove();
            }
        }, 8000);
    }
    
    showError(message) {
        this.addSystemMessage(message, 'error');
    }
    
    playNotificationSound(force = false) {
        if (!this.audioContext) return;
        if (this.isMuted && !force) return;
        
        try {
            // Resume context if suspended (mobile requirement)
            if (this.audioContext.state === 'suspended') {
                this.audioContext.resume();
            }
            
            // Simple beep sound
            const oscillator = this.audioContext.createOscillator();
            const gainNode = this.audioContext.createGain();
            
            oscillator.connect(gainNode);
            gainNode.connect(this.audioContext.destination);
            
            oscillator.frequency.value = 880;
            oscillator.type = 'sine';
            
            gainNode.gain.setValueAtTime(0.3, this.audioContext.currentTime);
            gainNode.gain.exponentialRampToValueAtTime(0.01, this.audioContext.currentTime + 0.2);
            
            oscillator.start(this.audioContext.currentTime);
            oscillator.stop(this.audioContext.currentTime + 0.2);
        } catch (e) {
            console.warn('Failed to play notification sound:', e);
        }
    }
    
    parseTimestamp(ts) {
        // Validate timestamp - if less than 1e12, assume seconds and convert to ms
        const num = parseInt(ts);
        return num < 1e12 ? num * 1000 : num;
    }
    
    /// Escapes text for interpolation into an innerHTML string.
    ///
    /// Only the quoted-reply block needs this, because it is the one place the
    /// client builds markup as a string. Everywhere else sets `textContent`,
    /// which does not interpret markup at all — passing escaped text to
    /// `textContent` was double-escaping it, so an animal name or reply preview
    /// containing `&` or `<` displayed as `&amp;` or `&lt;`.
    escapeHtml(text) {
        const div = document.createElement('div');
        div.textContent = text;
        return div.innerHTML;
    }
    
    formatTimestamp(ms) {
        const date = new Date(ms);
        return date.toLocaleTimeString([], { 
            hour: '2-digit', 
            minute: '2-digit',
            hour12: true 
        });
    }
    
    cleanup() {
        // Clear all timers and intervals
        if (this.cleanupIntervalId) {
            clearInterval(this.cleanupIntervalId);
            this.cleanupIntervalId = null;
        }
        if (this.typingTimeout) {
            clearTimeout(this.typingTimeout);
            this.typingTimeout = null;
        }
        if (this.scrollTimeoutId) {
            clearTimeout(this.scrollTimeoutId);
            this.scrollTimeoutId = null;
        }
        // Cancel a pending auto-scroll frame
        if (this.autoScrollFrameId) {
            cancelAnimationFrame(this.autoScrollFrameId);
            this.autoScrollFrameId = null;
        }
        if (this.historyLoadTimeoutId) {
            clearTimeout(this.historyLoadTimeoutId);
            this.historyLoadTimeoutId = null;
        }
        // Clear heartbeat interval
        if (this.heartbeatIntervalId) {
            clearInterval(this.heartbeatIntervalId);
            this.heartbeatIntervalId = null;
        }
        // Close WebSocket if open
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this.ws.close();
        }
    }
    
    updateHeartbeat() {
        const now = Date.now();
        const idleSeconds = Math.floor((now - this.lastActivityTime) / 1000);
        const aliveMinutes = Math.floor((now - this.roomStartTime) / 60000);
        
        // Update lifespan display
        if (aliveMinutes < 60) {
            this.roomLifespan.textContent = `${aliveMinutes}m alive`;
        } else {
            const hours = Math.floor(aliveMinutes / 60);
            const mins = aliveMinutes % 60;
            this.roomLifespan.textContent = `${hours}h ${mins}m alive`;
        }
        
        // Update heartbeat state based on activity (1 minute room timeout)
        this.roomHeartbeat.classList.remove('active', 'warning', 'critical');
        this.chat.classList.remove('room-fading');
        
        if (idleSeconds < 10) {
            // Active: recent messages (0-10s)
            this.roomHeartbeat.classList.add('active');
        } else if (idleSeconds < 30) {
            // Normal: 10-30s idle
        } else if (idleSeconds < 45) {
            // Warning: 30-45s idle - show warning
            this.roomHeartbeat.classList.add('warning');
            if (!this.fadeWarningShown) {
                this.fadeWarningShown = true;
                this.showFadeWarning();
            }
        } else {
            // Critical: 45+ seconds, room is dying (server deletes at 60s)
            this.roomHeartbeat.classList.add('critical');
            this.chat.classList.add('room-fading');
        }
    }
    
    showFadeWarning() {
        const existing = document.querySelector('.fade-warning');
        if (existing) existing.remove();
        
        const warning = document.createElement('div');
        warning.className = 'fade-warning';
        warning.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-hourglass"/></svg>Room fading soon... say something!';
        document.body.appendChild(warning);
        
        setTimeout(() => {
            if (warning.parentNode) warning.remove();
        }, 5000);
    }
}

// Initialize app when DOM is ready
if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
        new ChatApp();
    });
} else {
    new ChatApp();
}
