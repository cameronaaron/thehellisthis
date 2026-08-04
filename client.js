

const EMOJI_GROUPS = [
    {
        id: 'smileys',
        label: 'Smileys & people',
        icon: '😀',
        emoji: ['😀','😃','😄','😁','😆','😅','🤣','😂','🙂','🙃','🫠','😉','😊','😇','🥰','😍','🤩','😘','😗','😚','😙','🥲','😋','😛','😜','🤪','😝','🤑','🤗','🤭','🫢','🤫','🤔','🫡','🤐','🤨','😐','😑','😶','🫥','😏','😒','🙄','😬','🤥','😌','😔','😪','🤤','😴','😷','🤒','🤕','🤢','🤮','🤧','🥵','🥶','🥴','😵','🤯','🤠','🥳','🥸','😎','🤓','🧐','😕','🫤','😟','🙁','😮','😯','😲','😳','🥺','🥹','😦','😧','😨','😰','😥','😢','😭','😱','😖','😣','😞','😓','😩','😫','🥱','😤','😡','😠','🤬','😈','👿','💀','💩','🤡','👻','👽','🤖','🎃'],
    },
    {
        id: 'gestures',
        label: 'Gestures & body',
        icon: '👋',
        emoji: ['👋','🤚','🖐️','✋','🖖','🫱','🫲','🫳','🫴','👌','🤌','🤏','✌️','🤞','🫰','🤟','🤘','🤙','👈','👉','👆','🖕','👇','☝️','🫵','👍','👎','✊','👊','🤛','🤜','👏','🙌','🫶','👐','🤲','🤝','🙏','✍️','💅','🤳','💪','🦾','🦵','🦶','👂','👃','🧠','🫀','🫁','🦷','👀','👁️','👅','👄','🫦'],
    },
    {
        id: 'animals',
        label: 'Animals & nature',
        icon: '🦊',
        emoji: ['🐶','🐱','🐭','🐹','🐰','🦊','🐻','🐼','🐨','🐯','🦁','🐮','🐷','🐸','🐵','🙈','🙉','🙊','🐔','🐧','🐦','🐤','🦆','🦅','🦉','🦇','🐺','🐗','🐴','🦄','🐝','🪱','🐛','🦋','🐌','🐞','🐜','🪰','🦂','🐢','🐍','🦎','🦖','🐙','🦑','🦐','🦞','🦀','🐡','🐠','🐟','🐬','🐳','🐋','🦈','🐊','🐅','🦓','🦍','🦧','🐘','🦛','🐪','🦒','🦘','🐃','🐂','🐄','🐎','🐖','🐏','🐑','🦙','🐐','🦌','🐕','🐩','🦮','🐈','🐓','🦃','🦤','🦚','🦜','🦢','🦩','🕊️','🐇','🦝','🦨','🦡','🦫','🦦','🦥','🐁','🐀','🐿️','🦔','🌵','🎄','🌲','🌳','🌴','🌱','🌿','☘️','🍀','🎍','🪴','🎋','🍃','🍂','🍁','🍄','🐚','🪨','🌾','💐','🌷','🌹','🥀','🌺','🌸','🌼','🌻','🌞','🌝','🌛','🌜','🌚','🌕','🌙','⭐','🌟','✨','⚡','☄️','💥','🔥','🌪️','🌈','☀️','🌤️','⛅','☁️','🌧️','⛈️','🌩️','🌨️','❄️','☃️','⛄','💨','💧','💦','🌊'],
    },
    {
        id: 'food',
        label: 'Food & drink',
        icon: '🍕',
        emoji: ['🍏','🍎','🍐','🍊','🍋','🍌','🍉','🍇','🍓','🫐','🍈','🍒','🍑','🥭','🍍','🥥','🥝','🍅','🍆','🥑','🥦','🥬','🥒','🌶️','🫑','🌽','🥕','🫒','🧄','🧅','🥔','🍠','🥐','🥯','🍞','🥖','🥨','🧀','🥚','🍳','🧈','🥞','🧇','🥓','🥩','🍗','🍖','🌭','🍔','🍟','🍕','🫓','🥪','🥙','🧆','🌮','🌯','🫔','🥗','🥘','🫕','🥫','🍝','🍜','🍲','🍛','🍣','🍱','🥟','🦪','🍤','🍙','🍚','🍘','🍥','🥠','🥮','🍢','🍡','🍧','🍨','🍦','🥧','🧁','🍰','🎂','🍮','🍭','🍬','🍫','🍿','🍩','🍪','🌰','🥜','🍯','🥛','🍼','🫖','☕','🍵','🧃','🥤','🧋','🍶','🍺','🍻','🥂','🍷','🥃','🍸','🍹','🧉','🍾','🧊'],
    },
    {
        id: 'activity',
        label: 'Activity & travel',
        icon: '⚽',
        emoji: ['⚽','🏀','🏈','⚾','🥎','🎾','🏐','🏉','🥏','🎱','🪀','🏓','🏸','🏒','🏑','🥍','🏏','🪃','🥅','⛳','🪁','🏹','🎣','🤿','🥊','🥋','🎽','🛹','🛼','🛷','⛸️','🥌','🎿','⛷️','🏂','🪂','🏋️','🤼','🤸','⛹️','🤺','🤾','🏌️','🏇','🧘','🏄','🏊','🤽','🚣','🧗','🚴','🚵','🎪','🎭','🎨','🎬','🎤','🎧','🎼','🎹','🥁','🪘','🎷','🎺','🪗','🎸','🪕','🎻','🎲','♟️','🎯','🎳','🎮','🎰','🧩','🚗','🚕','🚙','🚌','🚎','🏎️','🚓','🚑','🚒','🚐','🛻','🚚','🚛','🚜','🦯','🦽','🦼','🛴','🚲','🛵','🏍️','🛺','🚨','🚔','🚍','🚘','🚖','🚡','🚠','🚟','🚃','🚋','🚞','🚝','🚄','🚅','🚈','🚂','🚆','🚇','🚊','🚉','✈️','🛫','🛬','🛩️','💺','🛰️','🚀','🛸','🚁','🛶','⛵','🚤','🛥️','🛳️','⛴️','🚢','⚓','🪝','⛽','🚧','🚦','🚥','🗺️','🗿','🗽','🗼','🏰','🏯','🏟️','🎡','🎢','🎠','⛲','⛱️','🏖️','🏝️','🏜️','🌋','⛰️','🏔️','🗻','🏕️','⛺','🏠','🏡','🏘️','🏚️','🏗️','🏭','🏢','🏬','🏣','🏤','🏥','🏦','🏨','🏪','🏫','🏩','💒','🏛️','⛪','🕌','🕍','🛕','🕋','⛩️','🌁','🌃','🏙️','🌄','🌅','🌆','🌇','🌉'],
    },
    {
        id: 'objects',
        label: 'Objects',
        icon: '💡',
        emoji: ['⌚','📱','💻','⌨️','🖥️','🖨️','🖱️','🕹️','💽','💾','💿','📀','📼','📷','📸','📹','🎥','📽️','📞','☎️','📟','📠','📺','📻','🎙️','⏱️','⏲️','⏰','🕰️','⌛','⏳','📡','🔋','🔌','💡','🔦','🕯️','🪔','🧯','🛢️','💸','💵','💴','💶','💷','🪙','💰','💳','💎','⚖️','🪜','🧰','🪛','🔧','🔨','⚒️','🛠️','⛏️','🪚','🔩','⚙️','🪤','🧱','⛓️','🧲','🔫','💣','🧨','🪓','🔪','🗡️','⚔️','🛡️','🚬','⚰️','🪦','⚱️','🏺','🔮','📿','🧿','💈','⚗️','🔭','🔬','🕳️','🩹','🩺','💊','💉','🩸','🧬','🦠','🧫','🧪','🌡️','🧹','🪣','🧼','🪥','🧽','🧴','🔑','🗝️','🚪','🪑','🛋️','🛏️','🛌','🧸','🪆','🖼️','🪞','🪟','🛍️','🛒','🎁','🎈','🎏','🎀','🪄','🪅','🎊','🎉','🎎','🏮','🎐','🧧','✉️','📩','📨','📧','💌','📥','📤','📦','🏷️','🪧','📪','📫','📬','📭','📮','📯','📜','📃','📄','📑','🧾','📊','📈','📉','🗒️','🗓️','📆','📅','🗑️','📇','🗃️','🗳️','🗄️','📋','📁','📂','🗂️','🗞️','📰','📓','📔','📒','📕','📗','📘','📙','📚','📖','🔖','🧷','🔗','📎','🖇️','📐','📏','🧮','📌','📍','✂️','🖊️','🖋️','✒️','🖌️','🖍️','📝','✏️','🔍','🔎','🔏','🔐','🔒','🔓'],
    },
    {
        id: 'symbols',
        label: 'Symbols',
        icon: '❤️',
        emoji: ['❤️','🧡','💛','💚','💙','💜','🖤','🤍','🤎','💔','❣️','💕','💞','💓','💗','💖','💘','💝','💟','☮️','✝️','☪️','🕉️','☸️','✡️','🔯','🕎','☯️','☦️','🛐','⛎','♈','♉','♊','♋','♌','♍','♎','♏','♐','♑','♒','♓','🆔','⚛️','🉑','☢️','☣️','📴','📳','🈶','🈚','🈸','🈺','🈷️','✴️','🆚','💮','🉐','㊙️','㊗️','🈴','🈵','🈹','🈲','🅰️','🅱️','🆎','🆑','🅾️','🆘','❌','⭕','🛑','⛔','📛','🚫','💯','💢','♨️','🚷','🚯','🚳','🚱','🔞','📵','🚭','❗','❕','❓','❔','‼️','⁉️','🔅','🔆','〽️','⚠️','🚸','🔱','⚜️','🔰','♻️','✅','🈯','💹','❇️','✳️','❎','🌐','💠','Ⓜ️','🌀','💤','🏧','🚾','♿','🅿️','🛗','🈳','🈂️','🛂','🛃','🛄','🛅','🚹','🚺','🚼','⚧️','🚻','🚮','🎦','📶','🈁','🔣','ℹ️','🔤','🔡','🔠','🆖','🆗','🆙','🆒','🆕','🆓','0️⃣','1️⃣','2️⃣','3️⃣','4️⃣','5️⃣','6️⃣','7️⃣','8️⃣','9️⃣','🔟','🔢','#️⃣','*️⃣','⏏️','▶️','⏸️','⏯️','⏹️','⏺️','⏭️','⏮️','⏩','⏪','🔀','🔁','🔂','◀️','🔼','🔽','⏫','⏬','➡️','⬅️','⬆️','⬇️','↗️','↘️','↙️','↖️','↕️','↔️','↪️','↩️','⤴️','⤵️','🔃','🔄','🔚','🔙','🔛','🔝','🔜','✔️','☑️','🔘','⚪','⚫','🔴','🔵','🟠','🟡','🟢','🟣','🟤','🔺','🔻','🔸','🔹','🔶','🔷','🔳','🔲','▪️','▫️','◾','◽','◼️','◻️','⬛','⬜','🟥','🟧','🟨','🟩','🟦','🟪','🟫','🔈','🔇','🔉','🔊','🔔','🔕','📣','📢','👁‍🗨','💬','💭','🗯️','♠️','♣️','♥️','♦️','🃏','🎴','🀄','🕐','🕑','🕒','🕓','🕔','🕕','🕖','🕗','🕘','🕙','🕚','🕛'],
    },
];

const QUICK_REACTIONS = ['👍', '❤️', '😂', '🎉', '😮', '😢', '🔥', '🙏'];

const REACTION_EMOJI = [
    '‼️', '✅', '❌', '❤️', '⭐', '🎉', '🎯', '👀', '👇', '👋', '👍', '👎', '💀', '💜', '💡', '💯',
    '🔥', '😀', '😂', '😅', '😍', '😎', '😐', '😔', '😡', '😢', '😭', '😮', '😱', '😴', '🙄', '🙏',
    '🚀', '🤔', '🤝', '🤣', '🥳', '🫡',
];

const MAX_ATTACHMENT_BYTES = 131072;

const IDLE_CLOSE_CODE = 4001;

const SUPERSEDED_CLOSE_CODE = 4002;

const REACTION_BAR_GRACE_MS = 400;

const REACTION_BAR_OVERLAP = 8;

const GROUPING_WINDOW_MS = 60000;

const MAIN_ROOM_NAME = 'main';

const MAIN_ROOM_FADE_SECONDS = 1800;

const IDLE_EVICTION_SECONDS = 600;

// The novachannel proof-of-concept room (server: config::NOVA_ROOM). Only
// this room ever imports /nova.js — every other room's bundle behavior is
// unchanged (mirrors the roomName === MAIN_ROOM_NAME precedent above).
const NOVA_ROOM_NAME = 'nova';

// How long an RLN rate-limit epoch lasts (server: config::NOVA_RLN_EPOCH_SECONDS).
// A second, different anonymous post from the same member inside one epoch
// recovers their identity secret — that's the mechanism, not a bug.
const NOVA_RLN_EPOCH_SECONDS = 30;

function isTouchDevice() {
    return window.matchMedia('(pointer: coarse)').matches;
}

const EMOJI_KEYWORDS = {
    '😀': 'grin smile happy', '😂': 'laugh cry joy lol', '🤣': 'rofl laugh lol',
    '🙂': 'smile', '😉': 'wink', '😍': 'love heart eyes', '🥰': 'love adore',
    '😘': 'kiss', '😎': 'cool sunglasses', '🤔': 'think hmm', '😐': 'neutral meh',
    '🙄': 'eyeroll annoyed', '😢': 'sad cry', '😭': 'sob cry bawl', '😡': 'angry mad rage',
    '😱': 'scream shock fear', '😴': 'sleep tired zzz', '🥳': 'party celebrate',
    '💀': 'skull dead dying', '👻': 'ghost boo', '🤖': 'robot bot ai',
    '👍': 'thumbs up yes approve like', '👎': 'thumbs down no disapprove',
    '👏': 'clap applause', '🙏': 'pray thanks please', '🤝': 'handshake deal agree',
    '💪': 'muscle strong flex', '👀': 'eyes look watch', '👋': 'wave hello hi bye',
    '❤️': 'heart love red', '💔': 'broken heart', '🔥': 'fire lit hot flame',
    '⭐': 'star', '✨': 'sparkle shiny', '⚡': 'lightning bolt zap fast',
    '🎉': 'party tada celebrate congrats', '🎊': 'confetti party', '🎁': 'gift present',
    '✅': 'check tick yes done', '❌': 'cross no wrong', '💯': 'hundred perfect',
    '🚀': 'rocket launch ship fast', '🐛': 'bug insect', '💡': 'idea lightbulb',
    '🍕': 'pizza food', '☕': 'coffee', '🍺': 'beer', '🎂': 'cake birthday',
    '🐶': 'dog puppy', '🐱': 'cat kitten', '🦊': 'fox', '🦄': 'unicorn',
    '🌈': 'rainbow', '☀️': 'sun sunny', '🌙': 'moon night', '❄️': 'snow cold',
    '💻': 'laptop computer code', '📱': 'phone mobile', '🔒': 'lock secure',
    '💰': 'money cash', '🎯': 'target bullseye goal', '🫡': 'salute yes sir',
};

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
        this.isLoadingHistory = true;
        this.cleanupIntervalId = null;
        this.scrollTimeoutId = null;
        this.autoScrollFrameId = null;
        this.programmaticScroll = false;
        this.historyLoadTimeoutId = null;

        this.roomStartTime = Date.now();
        this.lastActivityTime = Date.now();
        this.myLastSentTime = Date.now();
        this.lastMessageTime = 0;
        this.recentMessageCount = 0;
        this.lastSenderId = null;
        this.lastSentAt = 0;
        this.heartbeatIntervalId = null;
        this.fadeWarningShown = false;

        this.lastHeartbeatTime = Date.now();
        this.heartbeatTimeoutId = null;

        this.isMuted = localStorage.getItem('chatMuted') === 'true';

        this.replyingTo = null;

        this.pendingAttachment = null;
        this.reactionTarget = null;
        this.reactionBarFor = null;
        this.reactionBarHideId = null;
        this.awaitingManualRejoin = false;
        this.roster = [];
        this.unreadCount = 0;
        this.dragDepth = 0;
        this.lightboxReturnFocus = null;
        this.lastRenderedDay = null;
        this.renderedMessages = new Map();
        this.lastMessageEl = null;

        this.chat = document.getElementById('chat');
        this.input = document.getElementById('messageInput');
        this.sendButton = document.getElementById('sendBtn');
        this.userCountNumEl = document.getElementById('userCountNum');
        this.statusChip = document.getElementById('statusChip');
        this.typingIndicator = document.getElementById('typingIndicator');
        this.typingText = document.getElementById('typingText');
        this.statusText = document.getElementById('statusText');
        this.charCount = document.getElementById('charCount');
        this.novaAnonymousToggle = document.getElementById('novaAnonymousToggle');
        this.novaAnonymousToggleLabel = document.getElementById('novaAnonymousToggleLabel');
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
        this.attachBtn = document.getElementById('attachBtn');
        this.fileInput = document.getElementById('fileInput');
        this.attachmentTray = document.getElementById('attachmentTray');
        this.attachmentPreview = document.getElementById('attachmentPreview');
        this.attachmentName = document.getElementById('attachmentName');
        this.attachmentSize = document.getElementById('attachmentSize');
        this.attachmentRemove = document.getElementById('attachmentRemove');
        this.emojiBtn = document.getElementById('emojiBtn');
        this.emojiPanel = document.getElementById('emojiPanel');
        this.emojiSearch = document.getElementById('emojiSearch');
        this.emojiTabs = document.getElementById('emojiTabs');
        this.emojiGrid = document.getElementById('emojiGrid');
        this.emojiEmpty = document.getElementById('emojiEmpty');
        this.reactionBar = document.getElementById('reactionBar');
        this.lightbox = document.getElementById('lightbox');
        this.lightboxImage = document.getElementById('lightboxImage');
        this.lightboxClose = document.getElementById('lightboxClose');
        this.jumpLatest = document.getElementById('jumpLatest');
        this.jumpLatestCount = document.getElementById('jumpLatestCount');
        this.dropOverlay = document.getElementById('dropOverlay');
        this.participantsSheet = document.getElementById('participantsSheet');
        this.participantsList = document.getElementById('participantsList');
        this.participantsCount = document.getElementById('participantsCount');
        this.participantsClose = document.getElementById('participantsClose');
        this.roomTitle = document.getElementById('userCount');

        this.init();
    }

    init() {
        this.setupEventListeners();
        this.setupComposerExtras();
        this.buildEmojiPicker();
        this.initAudioContext();
        this.updateRoomName();
        this.connect();

        this.cleanupIntervalId = setInterval(() => this.cleanupTypingIndicators(), 1000);

        this.heartbeatIntervalId = setInterval(() => this.updateHeartbeat(), 1000);

        this.chat.addEventListener('scroll', (e) => {
            this.handleChatScroll();
        });

        window.addEventListener('beforeunload', () => this.cleanup());
        window.addEventListener('pagehide', () => this.cleanup());
    }

    setupEventListeners() {
        this.input.addEventListener('keydown', (e) => this.handleInputKeydown(e));
        this.input.addEventListener('input', () => this.handleInput());
        this.sendButton.addEventListener('click', () => this.sendMessage());
        this.sendButton.addEventListener('touchend', (e) => {
            e.preventDefault();
            this.sendMessage();
        });

        document.getElementById('exploreLink').addEventListener('click', (e) => {
            e.preventDefault();
            if (withinNavigationCooldown()) return;
            startNavigationCooldown();
            const rooms = ['mellow-forest', 'midnight-owl', 'coffee-talks', 'tech-minds', 'random-thoughts', 'chill-zone', 'late-night', 'creative-corner'];
            const room = rooms[Math.floor(Math.random() * rooms.length)];
            window.location.href = `/${room}`;
        });

        this.createRoomBtn.addEventListener('click', () => {
            this.showCreateRoomDialog();
        });

        const shareRoomBtn = document.getElementById('shareRoomBtn');
        if (shareRoomBtn) {
            shareRoomBtn.addEventListener('click', () => {
                this.shareRoom();
            });
        }

        const dismissBtn = document.getElementById('dismissBanner');
        if (dismissBtn) {
            dismissBtn.addEventListener('click', () => {
                this.welcomeBanner.style.display = 'none';
                localStorage.setItem('bannerDismissed', 'true');
            });
        }

        if (localStorage.getItem('bannerDismissed') === 'true') {
            this.welcomeBanner.style.display = 'none';
        }

        this.muteBtn.addEventListener('click', () => this.toggleMute());
        this.updateMuteButton();

        this.replyPreviewClose.addEventListener('click', () => this.cancelReply());

        if (!isTouchDevice()) this.input.focus();
    }

    toggleMute() {
        this.isMuted = !this.isMuted;
        localStorage.setItem('chatMuted', this.isMuted);
        this.updateMuteButton();

        if (this.isMuted) {
            this.addSystemMessage('Notifications muted 🔇', 'info');
        } else {
            this.addSystemMessage('Notifications unmuted 🔊', 'info');
            this.playNotificationSound(true);
        }
    }

    updateMuteButton() {
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

        this.isLoadingHistory = true;

        if (this.historyLoadTimeoutId) clearTimeout(this.historyLoadTimeoutId);
        this.historyLoadTimeoutId = setTimeout(() => this.finishHistoryLoad(), 5000);

        try {
            this.ws = new WebSocket(wsURL);

            this.ws.onopen = () => this.handleWebSocketOpen();
            this.ws.onclose = (e) => this.handleWebSocketClose(e);
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
        this.lastActivityTime = Date.now();
        this.myLastSentTime = Date.now();
        this.roomStartTime = Date.now();
        this.fadeWarningShown = false;
        if (this.reconnectAttempts > 0) {
            this.addSystemMessage('Reconnected successfully', 'success');
        }
        this.reconnectAttempts = 0;
        this.updateConnectionStatus('connected');
        this.sendButton.disabled = false;
        this.input.disabled = false;
        if (!isTouchDevice()) this.input.focus();
    }

    handleWebSocketClose(event) {
        console.log('WebSocket disconnected', event && event.code);
        if (this.heartbeatTimeoutId) clearTimeout(this.heartbeatTimeoutId);
        this.connected = false;
        this.updateConnectionStatus('disconnected');
        this.sendButton.disabled = true;
        this.input.disabled = true;

        if (event && event.code === IDLE_CLOSE_CODE) {
            this.handleIdleEviction();
            return;
        }

        if (event && event.code === SUPERSEDED_CLOSE_CODE) {
            this.handleSuperseded();
            return;
        }

        this.tryReconnect();
    }

    handleIdleEviction() {
        this.awaitingManualRejoin = true;
        this.reconnectAttempts = 0;
        this.updateConnectionStatus('idle');
        this.addSystemMessage('You went quiet, so the room let you go. Say something to rejoin.', 'info');

        this.input.disabled = false;
        this.sendButton.disabled = false;
    }

    handleSuperseded() {
        this.awaitingManualRejoin = true;
        this.reconnectAttempts = 0;
        this.updateConnectionStatus('idle');
        this.addSystemMessage('This room is open in another tab. Say something here to bring it back.', 'info');

        this.input.disabled = false;
        this.sendButton.disabled = false;
    }

    rejoinAfterIdle() {
        if (!this.awaitingManualRejoin || this.connected) return false;
        this.awaitingManualRejoin = false;
        this.roster = [];
        this.addSystemMessage('Rejoining...', 'info');
        this.connect();
        return true;
    }

    handleWebSocketError(error) {
        console.error('WebSocket error:', error);
    }

    handleWebSocketMessage(event) {
        try {

            this.lastHeartbeatTime = Date.now();
            if (this.heartbeatTimeoutId) clearTimeout(this.heartbeatTimeoutId);

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
            case 'Roster':
                this.roster = data.users;
                this.renderParticipants();
                break;
            case 'Heartbeat':
                break;
            case 'ReconnectToken':
                this.finishHistoryLoad();
                if (this.roomName === NOVA_ROOM_NAME) this.startNovaHandshake();
                break;
            case 'NovaHandshakeResponse':
                this.completeNovaHandshake(data.msg2);
                break;
            case 'Sealed':
                this.handleSealedEvent(data.data);
                break;
            case 'RlnRegistered':
                if (this.pendingRlnRegisterResolve) {
                    this.pendingRlnRegisterResolve(data);
                    this.pendingRlnRegisterResolve = null;
                }
                break;
            case 'RlnPathResponse':
                if (this.pendingRlnPathResolve) {
                    this.pendingRlnPathResolve(data);
                    this.pendingRlnPathResolve = null;
                }
                break;
            case 'NovaAnonymousMessage':
                this.renderNovaAnonymousMessage(data);
                break;
            case 'NovaRlnSlashed':
                this.addSystemMessage(
                    `A member posted twice in one rate-limit window — their identity secret was recovered: ${data.recovered_secret.slice(0, 16)}…`,
                    'error'
                );
                break;
            default:
                console.warn('Unknown event type:', data.type);
        }
    }

    // ---- nova room only: novachannel handshake and sealed transport -------
    //
    // Nowhere else in this file branches on room name for anything beyond
    // the idle-grace-period read at `updateHeartbeat()` — this is the one
    // other place, and it is entirely self-contained: every method here is
    // only ever called when `this.roomName === NOVA_ROOM_NAME`.

    async startNovaHandshake() {
        try {
            const mod = await import('/nova.js');
            await mod.default();
            this.novaModule = mod;
            this.novaClient = new mod.NovaClient();
            const msg1 = this.novaClient.startHandshake();
            this.ws.send(JSON.stringify({ type: 'NovaHandshakeInit', msg1 }));
        } catch (e) {
            console.error('nova: failed to start handshake', e);
            this.addSystemMessage('Secure channel setup failed.', 'error');
        }
    }

    completeNovaHandshake(msg2) {
        if (!this.novaClient) return;
        try {
            const msg3 = this.novaClient.completeHandshake(msg2);
            this.ws.send(JSON.stringify({ type: 'NovaHandshakeComplete', msg3 }));
            this.addSystemMessage('Secure channel established.', 'success');
            this.startNovaRlnRegistration();
        } catch (e) {
            console.error('nova: handshake failed', e);
            this.addSystemMessage('Secure channel setup failed.', 'error');
        }
    }

    // ---- nova room only: anonymous, rate-limited posting via RLN ----------
    //
    // Independent of the PQ channel above: an anonymous post doesn't need
    // *this connection's* sealed identity, only the room-wide RLN
    // membership tree (server: session/nova_rln.rs). Registration happens
    // once, right after the PQ handshake completes; a path is requested
    // fresh immediately before every anonymous post rather than cached —
    // see session/nova_rln.rs's module doc for why a cached path goes
    // stale the moment anyone else registers.

    async startNovaRlnRegistration() {
        try {
            if (!this.novaModule) return;
            this.novaRlnIdentity = new this.novaModule.NovaRlnIdentity();
            const commitment = this.novaRlnIdentity.commitment();
            const registered = await new Promise((resolve) => {
                this.pendingRlnRegisterResolve = resolve;
                this.sendEvent({ type: 'RlnRegister', commitment });
            });
            this.novaRlnLeafIndex = registered.leaf_index;
            if (this.novaAnonymousToggleLabel) {
                this.novaAnonymousToggleLabel.hidden = false;
            }
        } catch (e) {
            console.error('nova rln: registration failed', e);
        }
    }

    requestNovaRlnPath() {
        return new Promise((resolve) => {
            this.pendingRlnPathResolve = resolve;
            this.sendEvent({ type: 'RlnPathRequest', leaf_index: this.novaRlnLeafIndex });
        });
    }

    async sendNovaAnonymousMessage(text) {
        if (!this.novaRlnIdentity || this.novaRlnLeafIndex === undefined) {
            this.showError('Anonymous identity is not ready yet.');
            return;
        }
        try {
            const pathResponse = await this.requestNovaRlnPath();
            const epoch = BigInt(Math.floor(Date.now() / 1000 / NOVA_RLN_EPOCH_SECONDS));
            const proofJson = this.novaRlnIdentity.prove(
                JSON.stringify(pathResponse.path),
                epoch,
                text
            );
            const { proof, y, nullifier } = JSON.parse(proofJson);
            this.sendEvent({ type: 'RlnMessage', proof, y, nullifier, text });
            this.myLastSentTime = Date.now();
            this.input.value = '';
            this.charCount.textContent = '0';
            this.input.focus();
        } catch (e) {
            console.error('nova rln: failed to post anonymously', e);
            this.showError('Failed to post anonymously.');
        }
    }

    // An RLN-proven message renders with no sender at all — that omission
    // is the point of the proof, not a gap in the markup. Reuses the
    // `.message`/`.bubble` classes every other room's messages already
    // carry (and are already styled), plus one small label of its own.
    renderNovaAnonymousMessage(msg) {
        const row = document.createElement('div');
        row.className = 'message received nova-anonymous';
        row.dataset.messageId = msg.message_id;

        const bubble = document.createElement('div');
        bubble.className = 'bubble';

        const label = document.createElement('div');
        label.className = 'nova-anonymous-label';
        label.textContent = 'Anonymous · RLN-verified';
        bubble.appendChild(label);

        const content = document.createElement('div');
        content.className = 'message-content';
        content.innerHTML = msg.text;
        bubble.appendChild(content);

        row.appendChild(bubble);
        this.chat.appendChild(row);

        if (this.shouldAutoScroll) {
            this.scrollToBottom();
        }
    }

    handleSealedEvent(dataB64) {
        if (!this.novaClient) return;
        try {
            const plaintext = this.novaClient.open(dataB64);
            // `undefined` is a ratchet-control record with nothing to
            // deliver (nova-wasm/src/lib.rs's `open` doc) — Phase 1 never
            // sends one, but the client honors the type regardless.
            if (plaintext === undefined) return;
            this.handleServerEvent(JSON.parse(plaintext));
        } catch (e) {
            console.error('nova: failed to open sealed frame', e);
        }
    }

    // Sends `payload` (a plain object, not yet stringified) as this
    // connection's next frame — sealed first when nova's session is
    // established, exactly as sent otherwise. Every outgoing
    // `this.ws.send(...)` in the file goes through this rather than the
    // socket directly, so nova's transport is a property of *sending*, not
    // something every call site has to remember.
    sendEvent(payload) {
        if (this.novaClient && this.novaClient.isEstablished()) {
            try {
                const data = this.novaClient.seal(JSON.stringify(payload));
                this.ws.send(JSON.stringify({ type: 'Sealed', data }));
                return;
            } catch (e) {
                console.error('nova: failed to seal outgoing frame', e);
                return;
            }
        }
        this.ws.send(JSON.stringify(payload));
    }

    handleIncomingMessage(msg) {
        if (!msg || !msg.message_id) {
            console.warn('Invalid message received:', msg);
            return;
        }

        this.lastActivityTime = Date.now();
        this.fadeWarningShown = false;

        if (!this.isLoadingHistory && !this.isAtBottom() && msg.user_id !== this.myUserId) {
            this.unreadCount += 1;
            this.updateJumpLatest();
        }

        this.lastMessageTime = Date.now();

        const isSent = Boolean(this.myUserId) && msg.user_id === this.myUserId;

        const sentAt = this.parseTimestamp(msg.timestamp);
        const continuesRun =
            msg.user_id === this.lastSenderId &&
            Math.abs(sentAt - this.lastSentAt) < GROUPING_WINDOW_MS;
        this.lastSenderId = msg.user_id;
        this.lastSentAt = sentAt;

        this.renderMessage(msg, isSent, continuesRun);

        if (!this.isLoadingHistory && this.shouldAutoScroll) {
            this.scrollToBottom();
        }

        if (!isSent) {
            this.playNotificationSound();

            if (this.isAtBottom()) {
                this.sendReadReceipt(msg.message_id);
            }
        }
    }

    renderMessage(msg, isSent, continuesRun = false) {
        if (this.renderedMessages.has(msg.message_id)) {
            return;
        }

        const sentAt = this.parseTimestamp(msg.timestamp);
        const timestamp = this.formatTimestamp(sentAt);
        this.maybeRenderDateSeparator(sentAt);

        const previous = this.lastMessageEl;
        if (previous) {
            previous.classList.toggle('run-end', !continuesRun);
        }

        const row = document.createElement('div');
        row.className = `message ${isSent ? 'sent' : 'received'} run-end`;
        row.dataset.messageId = msg.message_id;
        row.dataset.userId = msg.user_id;
        row.setAttribute('role', 'article');
        row.setAttribute('aria-label', `Message from ${msg.animal_name} at ${timestamp}`);

        const stack = document.createElement('div');
        stack.className = 'stack';

        if (!isSent && !continuesRun) {
            const sender = document.createElement('div');
            sender.className = 'sender';
            sender.textContent = msg.animal_name;
            stack.appendChild(sender);
        }

        if (msg.reply_to) {
            const repliedTo = document.createElement('div');
            repliedTo.className = 'replied-to';
            repliedTo.innerHTML = `
                <div class="replied-to-author">${this.escapeHtml(msg.reply_to.author_name || 'Unknown')}</div>
                <div class="replied-to-text">${this.escapeHtml(msg.reply_to.preview_text || '')}</div>
            `;
            repliedTo.addEventListener('click', () => this.scrollToMessage(msg.reply_to.message_id));
            stack.appendChild(repliedTo);
        }

        const bubble = document.createElement('div');
        bubble.className = 'bubble';
        bubble.title = timestamp;

        if (msg.text.trim()) {
            const content = document.createElement('div');
            content.className = 'message-content';
            content.innerHTML = msg.text;
            bubble.appendChild(content);
        }

        if (msg.attachment) {
            bubble.classList.add('has-image');
            bubble.appendChild(this.renderAttachment(msg.attachment));
        }

        stack.appendChild(bubble);

        const reactions = document.createElement('div');
        reactions.className = 'reactions';
        for (const reaction of msg.reactions || []) {
            reactions.appendChild(
                this.buildReactionPill(msg.message_id, reaction.emoji, reaction.count, reaction.reacted)
            );
        }
        stack.appendChild(reactions);

        const receipt = document.createElement('div');
        receipt.className = 'receipt';
        receipt.textContent = timestamp;
        stack.appendChild(receipt);

        if (!isSent) {
            const avatar = document.createElement('div');
            avatar.className = 'avatar';
            avatar.setAttribute('aria-hidden', 'true');
            avatar.textContent = msg.animal_name[0].toUpperCase();
            row.appendChild(avatar);
        }

        row.appendChild(stack);

        const replyBtn = document.createElement('button');
        replyBtn.className = 'reply-btn';
        replyBtn.setAttribute('aria-label', `Reply to ${msg.animal_name}`);
        replyBtn.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-reply"/></svg>';
        replyBtn.addEventListener('click', (e) => {
            e.stopPropagation();
            this.startReply(msg.message_id, msg.animal_name, msg.text);
        });
        row.appendChild(replyBtn);

        this.chat.appendChild(row);
        this.renderedMessages.set(msg.message_id, row);
        this.lastMessageEl = row;
        this.pruneRenderedMessages();
    }

    setupComposerExtras() {
        this.attachBtn.addEventListener('click', () => this.fileInput.click());
        this.attachmentRemove.addEventListener('click', () => this.clearAttachment());
        this.fileInput.addEventListener('change', () => {
            const file = this.fileInput.files && this.fileInput.files[0];
            if (file) this.stageAttachment(file);
        });

        this.emojiBtn.addEventListener('click', () => this.toggleEmojiPanel());
        this.emojiSearch.addEventListener('input', () => this.renderEmojiGrid(this.emojiSearch.value));
        this.emojiSearch.addEventListener('keydown', (e) => {
            if (e.key === 'Escape') {
                e.preventDefault();
                this.toggleEmojiPanel(false);
                this.input.focus();
            }
        });

        document.addEventListener('click', (e) => {
            if (!this.emojiPanel.hidden
                && !this.emojiPanel.contains(e.target)
                && !this.emojiBtn.contains(e.target)) {
                this.toggleEmojiPanel(false);
            }
        });

        document.addEventListener('keydown', (e) => {
            if (e.key !== 'Escape') return;
            if (!this.lightbox.hidden) { this.closeLightbox(); return; }
            if (!this.emojiPanel.hidden) { this.toggleEmojiPanel(false); this.input.focus(); return; }
            if (!this.reactionBar.hidden) { this.hideReactionBar(); }
        });

        this.lightboxClose.addEventListener('click', () => this.closeLightbox());
        this.lightbox.addEventListener('click', (e) => {
            if (e.target === this.lightbox) this.closeLightbox();
        });

        this.roomTitle.addEventListener('click', () => this.toggleParticipants());
        this.participantsClose.addEventListener('click', () => this.toggleParticipants(false));
        document.addEventListener('click', (e) => {
            if (!this.participantsSheet.hidden
                && !this.participantsSheet.contains(e.target)
                && !this.roomTitle.contains(e.target)) {
                this.toggleParticipants(false);
            }
        });

        this.jumpLatest.addEventListener('click', () => {
            this.shouldAutoScroll = true;
            this.unreadCount = 0;
            this.scrollToBottom();
            this.updateJumpLatest();
        });

        this.chat.addEventListener('pointerover', (e) => {
            if (e.pointerType === 'touch') return;
            const message = e.target.closest('.message');
            if (message) {
                this.cancelReactionBarHide();
                this.showReactionBar(message);
            }
        });
        this.chat.addEventListener('pointerleave', (e) => {
            if (e.relatedTarget && this.reactionBar.contains(e.relatedTarget)) return;
            this.scheduleReactionBarHide();
        });
        this.reactionBar.addEventListener('pointerenter', () => this.cancelReactionBarHide());
        this.reactionBar.addEventListener('pointerleave', () => this.scheduleReactionBarHide());
        this.chat.addEventListener('scroll', () => this.hideReactionBar(), { passive: true });

        let pressTimer = null;
        this.chat.addEventListener('touchstart', (e) => {
            const message = e.target.closest('.message');
            if (!message) return;
            pressTimer = setTimeout(() => this.showReactionBar(message), 450);
        }, { passive: true });
        for (const event of ['touchend', 'touchmove', 'touchcancel']) {
            this.chat.addEventListener(event, () => {
                if (pressTimer) { clearTimeout(pressTimer); pressTimer = null; }
            }, { passive: true });
        }

        this.input.addEventListener('paste', (e) => {
            const items = e.clipboardData && e.clipboardData.items;
            if (!items) return;
            for (const item of items) {
                if (item.kind === 'file' && item.type.startsWith('image/')) {
                    e.preventDefault();
                    const file = item.getAsFile();
                    if (file) this.stageAttachment(file);
                    return;
                }
            }
        });

        window.addEventListener('dragenter', (e) => {
            if (!this.dragHasFiles(e)) return;
            e.preventDefault();
            this.dragDepth += 1;
            this.dropOverlay.hidden = false;
        });
        window.addEventListener('dragover', (e) => {
            if (this.dragHasFiles(e)) e.preventDefault();
        });
        window.addEventListener('dragleave', (e) => {
            if (!this.dragHasFiles(e)) return;
            this.dragDepth = Math.max(0, this.dragDepth - 1);
            if (this.dragDepth === 0) this.dropOverlay.hidden = true;
        });
        window.addEventListener('drop', (e) => {
            if (!this.dragHasFiles(e)) return;
            e.preventDefault();
            this.dragDepth = 0;
            this.dropOverlay.hidden = true;
            const file = e.dataTransfer.files && e.dataTransfer.files[0];
            if (file) this.stageAttachment(file);
        });
    }

    dragHasFiles(e) {
        return Boolean(e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files'));
    }

    async prepareAttachment(file) {
        if (!file || !file.type.startsWith('image/')) {
            this.showError('Only images can be attached');
            return null;
        }

        const bitmap = await this.decodeImage(file);
        if (!bitmap) {
            this.showError('That image could not be read');
            return null;
        }

        for (const maxEdge of [1600, 1280, 1024, 800, 640]) {
            for (const quality of [0.82, 0.7, 0.55]) {
                const encoded = await this.encodeBitmap(bitmap, maxEdge, quality);
                if (encoded && encoded.data.length <= MAX_ATTACHMENT_BYTES) {
                    if (bitmap.close) bitmap.close();
                    return encoded;
                }
            }
        }

        if (bitmap.close) bitmap.close();
        this.showError('That image is too large to send');
        return null;
    }

    async decodeImage(file) {
        if (window.createImageBitmap) {
            try {
                return await createImageBitmap(file);
            } catch (e) {
                console.warn('createImageBitmap failed, falling back', e);
            }
        }

        return new Promise((resolve) => {
            const url = URL.createObjectURL(file);
            const img = new Image();
            img.onload = () => { URL.revokeObjectURL(url); resolve(img); };
            img.onerror = () => { URL.revokeObjectURL(url); resolve(null); };
            img.src = url;
        });
    }

    async encodeBitmap(bitmap, maxEdge, quality) {
        const sourceW = bitmap.width;
        const sourceH = bitmap.height;
        if (!sourceW || !sourceH) return null;

        const scale = Math.min(1, maxEdge / Math.max(sourceW, sourceH));
        const width = Math.max(1, Math.round(sourceW * scale));
        const height = Math.max(1, Math.round(sourceH * scale));

        const canvas = document.createElement('canvas');
        canvas.width = width;
        canvas.height = height;
        const ctx = canvas.getContext('2d');
        if (!ctx) return null;
        ctx.drawImage(bitmap, 0, 0, width, height);

        let url = canvas.toDataURL('image/webp', quality);
        if (!url.startsWith('data:image/webp')) {
            url = canvas.toDataURL('image/jpeg', quality);
        }

        const match = /^data:([^;,]+);base64,(.*)$/.exec(url);
        if (!match) return null;

        return { mime: match[1], data: match[2], width, height, faded: false };
    }

    async stageAttachment(file) {
        if (this.pendingAttachment) {
            this.showError('One image at a time');
            return;
        }

        this.setAttachmentBusy(true);
        try {
            const attachment = await this.prepareAttachment(file);
            if (!attachment) return;

            this.pendingAttachment = attachment;
            this.attachmentPreview.src = this.attachmentDataUrl(attachment);
            this.attachmentName.textContent = file.name || 'image';
            this.attachmentSize.textContent = this.formatBytes(
                Math.floor(attachment.data.length * 3 / 4)
            );
            this.attachmentTray.hidden = false;
            this.input.focus();
        } finally {
            this.setAttachmentBusy(false);
        }
    }

    clearAttachment() {
        this.pendingAttachment = null;
        this.attachmentTray.hidden = true;
        this.attachmentPreview.removeAttribute('src');
        this.fileInput.value = '';
    }

    setAttachmentBusy(busy) {
        this.attachBtn.disabled = busy;
        this.attachBtn.setAttribute('aria-busy', busy ? 'true' : 'false');
    }

    attachmentDataUrl(attachment) {
        return `data:${attachment.mime};base64,${attachment.data}`;
    }

    formatBytes(bytes) {
        if (bytes < 1024) return `${bytes} B`;
        if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
        return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
    }

    renderAttachment(attachment) {
        const wrap = document.createElement('button');
        wrap.type = 'button';
        wrap.className = 'message-image-wrap';
        wrap.style.aspectRatio = `${attachment.width} / ${attachment.height}`;
        wrap.style.width = `${Math.min(320, attachment.width)}px`;

        if (attachment.faded) {
            wrap.classList.add('faded');
            wrap.disabled = true;
            wrap.style.cursor = 'default';
            const note = document.createElement('div');
            note.className = 'message-image-faded';
            note.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-broken-image"/></svg>';
            const label = document.createElement('span');
            label.textContent = 'Image faded';
            note.appendChild(label);
            wrap.appendChild(note);
            wrap.setAttribute('aria-label', 'An image that has faded from this room');
            return wrap;
        }

        const img = document.createElement('img');
        img.className = 'message-image';
        img.loading = 'lazy';
        img.decoding = 'async';
        img.alt = 'Shared image';
        img.src = this.attachmentDataUrl(attachment);
        wrap.appendChild(img);
        wrap.setAttribute('aria-label', 'Open image full size');
        wrap.addEventListener('click', () => this.openLightbox(img.src));
        return wrap;
    }

    openLightbox(src) {
        this.lightboxImage.src = src;
        this.lightbox.hidden = false;
        this.lightboxReturnFocus = document.activeElement;
        this.lightboxClose.focus();
    }

    closeLightbox() {
        this.lightbox.hidden = true;
        this.lightboxImage.removeAttribute('src');
        if (this.lightboxReturnFocus && this.lightboxReturnFocus.focus) {
            this.lightboxReturnFocus.focus();
        }
    }

    sendReaction(messageId, emoji) {
        if (!this.connected || this.ws.readyState !== WebSocket.OPEN) return;
        this.sendEvent({ type: 'React', message_id: messageId, emoji });
        this.hideReactionBar();
    }

    applyReaction({ message_id, user_id, emoji, active, count }) {
        const node = this.chat.querySelector(`[data-message-id="${message_id}"]`);
        if (!node) return;

        const container = node.querySelector('.reactions');
        if (!container) return;

        let pill = container.querySelector(`[data-emoji="${CSS.escape(emoji)}"]`);

        if (count === 0) {
            if (pill) pill.remove();
            return;
        }

        if (!pill) {
            pill = this.buildReactionPill(message_id, emoji, count, false);
            container.appendChild(pill);
        }

        pill.querySelector('.reaction-count').textContent = String(count);

        if (user_id === this.myUserId) {
            pill.classList.toggle('mine', active);
            pill.setAttribute('aria-pressed', active ? 'true' : 'false');
        }
    }

    buildReactionPill(messageId, emoji, count, mine) {
        const pill = document.createElement('button');
        pill.type = 'button';
        pill.className = `reaction-pill${mine ? ' mine' : ''}`;
        pill.dataset.emoji = emoji;
        pill.setAttribute('aria-pressed', mine ? 'true' : 'false');
        pill.setAttribute('aria-label', `React with ${emoji}`);

        const face = document.createElement('span');
        face.className = 'reaction-emoji';
        face.textContent = emoji;

        const number = document.createElement('span');
        number.className = 'reaction-count';
        number.textContent = String(count);

        pill.append(face, number);
        pill.addEventListener('click', () => this.sendReaction(messageId, emoji));
        return pill;
    }

    showReactionBar(messageNode) {
        if (!this.connected) return;
        const messageId = messageNode.dataset.messageId;
        if (!messageId) return;

        if (this.reactionBarFor === messageId && !this.reactionBar.hidden) return;
        this.reactionBarFor = messageId;

        this.reactionBar.replaceChildren();
        for (const emoji of QUICK_REACTIONS) {
            const button = document.createElement('button');
            button.type = 'button';
            button.textContent = emoji;
            button.setAttribute('aria-label', `React with ${emoji}`);
            button.addEventListener('click', (e) => {
                e.stopPropagation();
                this.sendReaction(messageId, emoji);
            });
            this.reactionBar.appendChild(button);
        }

        const more = document.createElement('button');
        more.type = 'button';
        more.innerHTML = '<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-add-reaction"/></svg>';
        more.setAttribute('aria-label', 'More reactions');
        more.addEventListener('click', (e) => {
            e.stopPropagation();
            this.openReactionPicker(messageId);
        });
        this.reactionBar.appendChild(more);

        this.reactionBar.hidden = false;

        const rect = messageNode.getBoundingClientRect();
        const barRect = this.reactionBar.getBoundingClientRect();
        const top = Math.max(8, rect.top - barRect.height + REACTION_BAR_OVERLAP);
        const left = Math.min(
            Math.max(8, rect.left),
            window.innerWidth - barRect.width - 8
        );
        this.reactionBar.style.top = `${top}px`;
        this.reactionBar.style.left = `${left}px`;
    }

    hideReactionBar() {
        this.cancelReactionBarHide();
        this.reactionBar.hidden = true;
        this.reactionBarFor = null;
    }

    scheduleReactionBarHide() {
        this.cancelReactionBarHide();
        this.reactionBarHideId = setTimeout(() => this.hideReactionBar(), REACTION_BAR_GRACE_MS);
    }

    cancelReactionBarHide() {
        if (this.reactionBarHideId) {
            clearTimeout(this.reactionBarHideId);
            this.reactionBarHideId = null;
        }
    }

    openReactionPicker(messageId) {
        this.reactionTarget = messageId;
        this.hideReactionBar();
        this.toggleEmojiPanel(true);
        this.renderEmojiGrid('');
    }

    buildEmojiPicker() {
        this.emojiTabs.replaceChildren();
        for (const group of EMOJI_GROUPS) {
            const tab = document.createElement('button');
            tab.type = 'button';
            tab.className = 'emoji-tab';
            tab.textContent = group.icon;
            tab.title = group.label;
            tab.setAttribute('role', 'tab');
            tab.setAttribute('aria-label', group.label);
            tab.setAttribute('aria-selected', 'false');
            tab.dataset.group = group.id;
            tab.addEventListener('click', () => this.scrollToEmojiGroup(group.id));
            this.emojiTabs.appendChild(tab);
        }
        this.renderEmojiGrid('');
    }

    renderEmojiGrid(query) {
        const needle = query.trim().toLowerCase();
        this.emojiGrid.replaceChildren();
        let shown = 0;

        const groups = this.reactionTarge
            ? [{ id: 'reactions', label: 'React with', emoji: REACTION_EMOJI }]
            : EMOJI_GROUPS;
        this.emojiTabs.hidden = Boolean(this.reactionTarget);

        for (const group of groups) {
            const matches = needle
                ? group.emoji.filter((e) => this.emojiMatches(e, needle))
                : group.emoji;
            if (!matches.length) continue;

            const label = document.createElement('div');
            label.className = 'emoji-group-label';
            label.textContent = group.label;
            label.dataset.groupLabel = group.id;
            this.emojiGrid.appendChild(label);

            for (const emoji of matches) {
                const cell = document.createElement('button');
                cell.type = 'button';
                cell.className = 'emoji-cell';
                cell.textContent = emoji;
                cell.setAttribute('role', 'option');
                cell.setAttribute('aria-label', emoji);
                cell.addEventListener('click', () => this.pickEmoji(emoji));
                this.emojiGrid.appendChild(cell);
                shown += 1;
            }
        }

        this.emojiEmpty.hidden = shown > 0;
    }

    emojiMatches(emoji, needle) {
        const keywords = EMOJI_KEYWORDS[emoji];
        return Boolean(keywords && keywords.includes(needle));
    }

    scrollToEmojiGroup(groupId) {
        const label = this.emojiGrid.querySelector(`[data-group-label="${groupId}"]`);
        if (label) label.scrollIntoView({ block: 'start' });
        for (const tab of this.emojiTabs.children) {
            tab.setAttribute('aria-selected', tab.dataset.group === groupId ? 'true' : 'false');
        }
    }

    pickEmoji(emoji) {
        if (this.reactionTarget) {
            this.sendReaction(this.reactionTarget, emoji);
            this.reactionTarget = null;
            this.toggleEmojiPanel(false);
            return;
        }
        this.insertEmoji(emoji);
    }

    insertEmoji(emoji) {
        const input = this.input;
        const start = input.selectionStart ?? input.value.length;
        const end = input.selectionEnd ?? input.value.length;

        input.value = input.value.slice(0, start) + emoji + input.value.slice(end);
        const caret = start + emoji.length;
        input.setSelectionRange(caret, caret);
        input.focus();
        this.handleInput();
    }

    toggleEmojiPanel(force) {
        const open = force ?? this.emojiPanel.hidden;
        this.emojiPanel.hidden = !open;
        this.emojiBtn.setAttribute('aria-expanded', open ? 'true' : 'false');

        if (open) {
            this.emojiSearch.value = '';
            this.renderEmojiGrid('');
            this.emojiSearch.focus();
        } else {
            this.reactionTarget = null;
            this.emojiTabs.hidden = false;
        }
    }

    updateJumpLatest() {
        const away = !this.isAtBottom();
        this.jumpLatest.hidden = !away;

        if (!away) {
            this.unreadCount = 0;
        }

        this.jumpLatestCount.hidden = this.unreadCount === 0;
        this.jumpLatestCount.textContent = String(Math.min(this.unreadCount, 99));
    }

    maybeRenderDateSeparator(sentAt) {
        const day = new Date(sentAt);
        if (Number.isNaN(day.getTime())) return;

        const key = day.toDateString();
        if (key === this.lastRenderedDay) return;
        this.lastRenderedDay = key;

        const today = new Date().toDateString();
        const yesterday = new Date(Date.now() - 86400000).toDateString();

        let label;
        if (key === today) {
            label = 'Today';
        } else if (key === yesterday) {
            label = 'Yesterday';
        } else {
            label = day.toLocaleDateString([], {
                month: 'short',
                day: 'numeric',
                year: day.getFullYear() === new Date().getFullYear() ? undefined : 'numeric',
            });
        }

        const rule = document.createElement('div');
        rule.className = 'date-separator';
        rule.setAttribute('role', 'separator');
        rule.textContent = label;
        this.chat.appendChild(rule);
    }

    pruneRenderedMessages() {
        const excess = this.renderedMessages.size - this.maxMessages;
        if (excess <= 0) return;

        const oldest = this.renderedMessages.entries();
        for (let i = 0; i < excess; i++) {
            const [id, el] = oldest.next().value;
            el.remove();
            this.renderedMessages.delete(id);
        }

        for (const rule of this.chat.querySelectorAll('.date-separator')) {
            const next = rule.nextElementSibling;
            if (!next || next.classList.contains('date-separator')) {
                rule.remove();
            }
        }
    }

    handleSystemEvent(event) {
        if (!event) return;

        if (event.UserJoined) {
            if (event.UserJoined.user_id === this.myUserId) {
                this.addSystemMessage(`You joined as ${event.UserJoined.animal_name} 🎉`, 'success');
            } else {
                this.addSystemMessage(`${event.UserJoined.animal_name} joined`, 'success');
            }
            this.updateRosterFrom(event.UserJoined.animal_name, true);
        } else if (event.UserLeft) {
            this.typingUsers.delete(event.UserLeft.animal_name);
            this.updateTypingIndicator();
            this.addSystemMessage(`${event.UserLeft.animal_name} left`);
            this.updateRosterFrom(event.UserLeft.animal_name, false);
        } else if (event.Typing) {
            this.handleTypingIndicator(event.Typing.animal_name, event.Typing.is_typing);
        } else if (event.Reaction) {
            this.applyReaction(event.Reaction);
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
        this.rejoinAfterIdle();

        if (e.key === 'Enter' && !e.shiftKey) {
            e.preventDefault();
            this.sendMessage();
        }
    }

    handleInput() {
        const text = this.input.value;
        this.charCount.textContent = text.length;

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
        if (this.rejoinAfterIdle()) {
            return;
        }

        const text = this.input.value.trim();

        if (!text && !this.pendingAttachment) {
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

        if (
            this.roomName === NOVA_ROOM_NAME &&
            this.novaAnonymousToggle &&
            this.novaAnonymousToggle.checked
        ) {
            this.sendNovaAnonymousMessage(text);
            return;
        }

        try {
            const messagePayload = { type: 'Message', text };

            if (this.pendingAttachment) {
                messagePayload.attachment = this.pendingAttachment;
            }

            if (this.replyingTo) {
                messagePayload.reply_to = {
                    message_id: this.replyingTo.messageId,
                    author_name: this.replyingTo.authorName,
                    preview_text: this.replyingTo.text.slice(0, 100)
                };
            }

            this.sendEvent(messagePayload);
            this.myLastSentTime = Date.now();
            this.input.value = '';
            this.clearAttachment();
            this.toggleEmojiPanel(false);
            this.charCount.textContent = '0';
            this.isCurrentlyTyping = false;
            this.cancelReply();
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
                this.sendEvent({ type: 'Typing', is_typing });
            } catch (e) {
                console.warn('Failed to send typing indicator:', e);
            }
        }
    }

    sendReadReceipt(message_id) {
        if (this.connected && message_id && this.ws.readyState === WebSocket.OPEN) {
            try {
                this.sendEvent({ type: 'ReadReceipt', message_id });
            } catch (e) {
                console.warn('Failed to send read receipt:', e);
            }
        }
    }

    scrollToBottom() {
        if (this.autoScrollFrameId) return;

        this.autoScrollFrameId = requestAnimationFrame(() => {
            this.autoScrollFrameId = null;
            this.programmaticScroll = true;
            this.chat.scrollTop = this.chat.scrollHeight;
            requestAnimationFrame(() => { this.programmaticScroll = false; });
        });
    }

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
        if (this.programmaticScroll || this.isLoadingHistory) return;

        const wasAtBottom = this.shouldAutoScroll;
        this.shouldAutoScroll = this.isAtBottom();
        this.updateJumpLatest();

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

    toggleParticipants(force) {
        const open = force ?? this.participantsSheet.hidden;
        this.participantsSheet.hidden = !open;
        this.roomTitle.setAttribute('aria-expanded', open ? 'true' : 'false');

        if (open) {
            this.requestRoster();
            this.participantsClose.focus();
        }
    }

    requestRoster() {
        if (!this.connected || this.ws.readyState !== WebSocket.OPEN) return;
        this.sendEvent({ type: 'RequestRoster' });
    }

    renderParticipants() {
        this.participantsCount.textContent =
            this.roster.length === 1 ? '1 here' : `${this.roster.length} here`;

        this.participantsList.replaceChildren();
        for (const name of this.roster) {
            const row = document.createElement('li');
            row.className = 'participant';

            const avatar = document.createElement('span');
            avatar.className = 'participant-avatar';
            avatar.setAttribute('aria-hidden', 'true');
            avatar.textContent = name[0].toUpperCase();

            const label = document.createElement('span');
            label.className = 'participant-name';
            label.textContent = name;

            row.append(avatar, label);

            if (name === this.myAnimalName) {
                row.classList.add('is-me');
                const you = document.createElement('span');
                you.className = 'participant-you';
                you.textContent = 'you';
                row.appendChild(you);
            }

            this.participantsList.appendChild(row);
        }
    }

    updateRosterFrom(name, arrived) {
        if (this.participantsSheet.hidden) return;

        if (arrived && !this.roster.includes(name)) {
            this.roster = [...this.roster, name].sort();
        } else if (!arrived) {
            this.roster = this.roster.filter((n) => n !== name);
        }
        this.renderParticipants();
    }

    updateConnectionStatus(status) {
        this.statusChip.hidden = false;
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
            case 'idle':
                this.statusText.textContent = 'Idle';
                break;
        }

        this.statusChip.title = this.statusText.textContent;
    }

    addSystemMessage(text, type = 'info') {
        const div = document.createElement('div');
        div.className = `system-message ${type}`;
        div.setAttribute('role', 'status');
        div.textContent = text;
        this.chat.appendChild(div);

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
            if (this.audioContext.state === 'suspended') {
                this.audioContext.resume();
            }

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
        const num = parseInt(ts);
        return num < 1e12 ? num * 1000 : num;
    }

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
        if (this.autoScrollFrameId) {
            cancelAnimationFrame(this.autoScrollFrameId);
            this.autoScrollFrameId = null;
        }
        if (this.historyLoadTimeoutId) {
            clearTimeout(this.historyLoadTimeoutId);
            this.historyLoadTimeoutId = null;
        }
        if (this.heartbeatIntervalId) {
            clearInterval(this.heartbeatIntervalId);
            this.heartbeatIntervalId = null;
        }
        if (this.ws && this.ws.readyState === WebSocket.OPEN) {
            this.ws.close();
        }
    }

    updateHeartbeat() {
        const now = Date.now();
        const isMain = this.roomName === MAIN_ROOM_NAME;
        const idleSeconds = isMain
            ? Math.floor((now - this.lastActivityTime) / 1000)
            : Math.floor((now - this.myLastSentTime) / 1000);
        const gracePeriod = isMain ? MAIN_ROOM_FADE_SECONDS : IDLE_EVICTION_SECONDS;
        const aliveMinutes = Math.floor((now - this.roomStartTime) / 60000);

        if (aliveMinutes < 60) {
            this.roomLifespan.textContent = `${aliveMinutes}m alive`;
        } else {
            const hours = Math.floor(aliveMinutes / 60);
            const mins = aliveMinutes % 60;
            this.roomLifespan.textContent = `${hours}h ${mins}m alive`;
        }

        this.roomHeartbeat.classList.remove('active', 'warning', 'critical');
        this.chat.classList.remove('room-fading');

        if (idleSeconds < 60) {
            this.roomHeartbeat.classList.add('active');
        } else if (idleSeconds < gracePeriod * 0.7) {
        } else if (idleSeconds < gracePeriod * 0.9) {
            this.roomHeartbeat.classList.add('warning');
            if (!this.fadeWarningShown) {
                this.fadeWarningShown = true;
                this.showFadeWarning(isMain);
            }
        } else {
            this.roomHeartbeat.classList.add('critical');
            if (isMain) this.chat.classList.add('room-fading');
        }
    }

    showFadeWarning(isMain) {
        const existing = document.querySelector('.fade-warning');
        if (existing) existing.remove();

        const warning = document.createElement('div');
        warning.className = 'fade-warning';
        const message = isMain
            ? "main's older messages are fading... say something to keep them!"
            : "you've gone quiet... say something or you'll be moved along";
        warning.innerHTML = `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true"><use href="#i-hourglass"/></svg>${message}`;
        document.body.appendChild(warning);

        setTimeout(() => {
            if (warning.parentNode) warning.remove();
        }, 5000);
    }
}

const NAV_COOLDOWN_MS = 800;

const NAV_COOLDOWN_KEY = 'navCooldownUntil';

function withinNavigationCooldown() {
    const until = Number(sessionStorage.getItem(NAV_COOLDOWN_KEY) || 0);
    return Date.now() < until;
}

function startNavigationCooldown() {
    sessionStorage.setItem(NAV_COOLDOWN_KEY, String(Date.now() + NAV_COOLDOWN_MS));
}

function debounceNavigation(selector) {
    const link = document.querySelector(selector);
    if (!link) return;
    link.addEventListener('click', (event) => {
        if (withinNavigationCooldown()) {
            event.preventDefault();
            return;
        }
        startNavigationCooldown();
    });
}

if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', () => {
        new ChatApp();
        debounceNavigation('.nav-back');
    });
} else {
    new ChatApp();
    debounceNavigation('.nav-back');
}
