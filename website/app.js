/* SITE_CONFIG — Update WhatsApp number and sales email before deploying to Cloudflare Pages. */
const SITE_CONFIG = {
  brandAr: "اطبع",
  brandEn: "Itbaa",
  whatsappNumber: "201007781842",
  salesEmail: "itbaa.odoo@gmail.com",
  defaultLang: "ar",
};

const QUEUE_SAMPLE = [
  {
    id: "POS-9482",
    printer: "EPSON TM-T88VII · 192.168.1.44",
    typeAr: "إيصال نقطة بيع 80مم",
    typeEn: "POS receipt 80mm",
    protocol: "ESC/POS 9100",
    ms: "142ms",
    status: "printed",
  },
  {
    id: "KDS-3107",
    printer: "XPrinter Kitchen · 192.168.1.51",
    typeAr: "تذكرة مطبخ حرارية",
    typeEn: "Thermal kitchen ticket",
    protocol: "ESC/POS 9100",
    ms: "118ms",
    status: "printed",
  },
  {
    id: "INV-2041",
    printer: "HP LaserJet Pro · WinSpool",
    typeAr: "فاتورة ضريبية A4",
    typeEn: "Tax invoice A4",
    protocol: "PDFium GDI",
    ms: "286ms",
    status: "queued",
  },
];

const I18N = {
  ar: {
    skip: "تخطي إلى المحتوى",
    navCompare: "قبل وبعد",
    navBenefits: "المزايا",
    navSteps: "خطوات التشغيل",
    navCompat: "التوافق",
    navFaq: "الأسئلة الشائعة",
    ctaPrimary: "اطلب تجربة مجانية",
    proofPill: "يعمل أوفلاين بالكامل · متوافق مع Odoo 17 إلى 20",
    heroH1: "طباعة صامتة ومباشرة لنظام أودو\nبدون IoT Box أو نوافذ طباعة",
    heroSub: "اربط طابعات الكاشير والمطبخ والباركود وفواتير A4 مباشرة من ويندوز وأندرويد عبر طابور محلي موثوق وترخيص أوفلاين.",
    heroNote: "توزيع خاص للعملاء المعتمدين · بدون بطاقة بنكية للتجربة",
    queueTitle: "طابور اطبع المحلي · SQLite WAL",
    queueAll: "الكل",
    queuePrinted: "مطبوع",
    queueQueued: "في الانتظار",
    statusPrinted: "تمت الطباعة",
    statusQueued: "جاهز للطباعة",
    compareKicker: "لماذا اطبع",
    compareH2: "استغنِ عن جهاز IoT Box ورسوم الصيانة الشهرية",
    compareSub: "قارن بين الطباعة التقليدية في المتصفح وتشغيل اطبع على نفس أجهزة الكاشير والمستودع.",
    beforeTitle: "الطريقة التقليدية أو IoT Box",
    before1: "شراء وتوصيل جهاز IoT Box لكل فرع مع صيانة مستمرة للشبكة",
    before2: "ظهور نافذة طباعة المتصفح في كل فاتورة وتباطؤ حركة الكاشير",
    before3: "ضياع مهام الطباعة عند انقطاع الشبكة أو إعادة تشغيل الجهاز",
    before4: "صعوبة تشغيل طابعات الملصقات ZPL أو الفواتير A4 بجانب الإيصالات",
    afterTitle: "مع اطبع على ويندوز وأندرويد",
    after1: "يعمل مباشرة على جهاز الكاشير أو التابلت الحالي بدون أي أجهزة إضافية",
    after2: "طباعة صامتة فورية بضغطة واحدة من نقاط البيع وتقارير أودو",
    after3: "طابور SQLite WAL محلي يمنع التكرار ويعيد المحاولة تلقائياً عند العطل المؤقت",
    after4: "دعم شامل لإيصالات 80مم وملصقات الشحن ZPL/TSPL وفواتير A4",
    tagline: "صُمم اطبع ليعمل بصمت في ذروة المبيعات، بطابور محلي لا يضيع فاتورة واحدة، وترخيص أوفلاين لا يتوقف بانقطاع الإنترنت.",
    benefitsKicker: "فوائد عملية",
    benefitsH2: "أربع ركائز تحمي سرعة الكاشير والمستودع",
    b1Title: "طابور SQLite WAL مضمون الترتيب",
    b1Desc: "مفتاح منع تكرار لكل مهمة وتسلسل FIFO مستقل لكل طابعة مع استعادة آمنة بعد انقطاع الكهرباء.",
    b2Title: "ترخيص Ed25519 أوفلاين بدون سيرفر",
    b2Desc: "تحقق رقمي محلي في أقل من 1 مللي ثانية مرتبط بدومين أودو الخاص بك دون الحاجة لاتصال خارجي.",
    b3Title: "بروتوكولات متعددة في تطبيق واحد",
    b3Desc: "طباعة مباشرة عبر TCP 9100 وIPP وBluetooth وUSB وطابعات ويندوز وPDFium بدقة عالية.",
    b4Title: "إدارة مركزية عبر Windows GPO",
    b4Desc: "قفل عنوان أودو ومفتاح التفعيل عبر سياسات المجموعة في ويندوز مع بقاء الرخصة ثابتة بعد التحديثات.",
    stepsKicker: "خطوات التشغيل",
    stepsH2: "جاهز للطباعة في ثلاث خطوات واضحة",
    s1Title: "اطلب الحزمة والتجربة",
    s1Desc: "أرسل بيانات شركتك وإصدار أودو لنجهز لك رابط الحزمة الخاصة ومفتاح التفعيل التجريبي.",
    s2Title: "ثبّت الموديول والتطبيق",
    s2Desc: "ارفع موديول أودو وثبّت تطبيق اطبع على ويندوز أو أندرويد ثم أدخل عنوان أودو ومفتاح التفعيل.",
    s3Title: "اطبع بصمت من أي شاشة",
    s3Desc: "حدد طابعة الإيصالات أو المطبخ أو الفواتير A4 لتخرج المطبوعات فوراً بدون نوافذ متصفح.",
    compatKicker: "مصفوفة التوافق",
    compatH2: "يعمل مع بيئة أودو الحالية لديك",
    thComponent: "المكوّن",
    thSupport: "الإصدارات والأنظمة المدعومة",
    thDetails: "تفاصيل التشغيل",
    c1Name: "إصدارات Odoo",
    c1Sup: "Odoo 17 · 18 · 19 · 20",
    c1Det: "دعم نقاط البيع POS وتقارير PDF وملصقات الباركود",
    c2Name: "نظام Windows",
    c2Sup: "Windows 10 و Windows 11",
    c2Det: "تطبيق خفيف مبني بـ Rust وTauri مع دعم GPO",
    c3Name: "نظام Android",
    c3Sup: "Android 8 إلى Android 15",
    c3Det: "تطبيق أصلي بلغة Kotlin لأجهزة الكاشير والتابلت",
    c4Name: "البروتوكولات والطابعات",
    c4Sup: "ESC/POS · ZPL · TSPL · IPP · PDF",
    c4Det: "شبكة LAN TCP 9100 وUSB وBluetooth وWindows Spooler",
    faqKicker: "الأسئلة الشائعة",
    faqH2: "إجابات مباشرة قبل البدء",
    f1q: "هل أحتاج إلى جهاز Odoo IoT Box لتشغيل اطبع؟",
    f1a: "لا، اطبع يلغي الحاجة تماماً إلى IoT Box ويعمل مباشرة على جهاز الكاشير أو تابلت أندرويد.",
    f2q: "هل يعمل نظام الترخيص إذا انقطع الإنترنت عن الفرع؟",
    f2a: "نعم، التفعيل يعتمد على توقيع Ed25519 محلي بالكامل ولا يتصل بأي سيرفر خارجي أثناء الطباعة.",
    f3q: "كيف أستلم ملفات التثبيت بعد طلب التجربة؟",
    f3a: "للحفاظ على أمان الكود والعملاء، يتم تسليم الحزمة بروابط خاصة ومفتاح تفعيل تجريبي فور مراجعة طلبك.",
    f4q: "هل يدعم اطبع الطباعة باللغة العربية في الإيصالات؟",
    f4a: "نعم، يدعم طباعة الإيصالات العربية والباركود والفواتير الضريبية بدقة كاملة عبر محرك PDFium والطباعة الحرارية.",
    f5q: "ماذا يحدث إذا فُصلت الطابعة أثناء إرسال الفاتورة؟",
    f5a: "يحفظ الطابور المحلي SQLite WAL المهمة ويعيد المحاولة تلقائياً للأعطال المؤقتة بدون طباعة مكررة.",
    f6q: "هل تبقى الرخصة مفعّلة عند تحديث تطبيق ويندوز أو أندرويد؟",
    f6a: "نعم، يُحفظ مفتاح الرخصة خارج مجلد التثبيت في إعدادات النظام أو سياسات GPO فيستمر العمل بعد أي تحديث.",
    f7q: "هل يمكن ربط أكثر من طابعة في نفس الفرع؟",
    f7a: "نعم، يدعم توجيه الإيصالات لطابعة الكاشير وتذاكر المطبخ لطابعة المطبخ والفواتير A4 لطابعة الليزر في آن واحد.",
    f8q: "هل يعمل مع Odoo Online أو Odoo.sh أو الاستضافة الخاصة؟",
    f8a: "يعمل مع Odoo.sh والاستضافة الخاصة On Premise وكل بيئة تدعم تثبيت موديولات أودو المخصصة.",
    reqKicker: "ابدأ التجربة",
    reqH2: "اطلب تجربة مجانية لشركتك",
    reqSub: "أدخل بياناتك الأساسية لفتح محادثة واتساب أو بريد إلكتروني جاهز بمواصفات بيئة أودو لديك.",
    reqRisk: "تجربة عملية كاملة · تسليم بروابط خاصة · دعم مباشر للتركيب",
    lblName: "الاسم الكامل",
    lblEmail: "البريد الإلكتروني للعمل",
    lblCompany: "اسم الشركة أو المتجر",
    lblOdoo: "إصدار أودو",
    lblChannel: "قناة التواصل المفضلة",
    chWhatsapp: "واتساب مباشر",
    chEmail: "البريد الإلكتروني",
    submitBtn: "اطلب تجربة مجانية",
    footerCopy: "اطبع · جميع الحقوق محفوظة",
    footerPrivacy: "سياسة الخصوصية",
    footerTerms: "الشروط والأحكام",
    errRequired: "هذا الحقل مطلوب",
    errEmail: "أدخل بريداً إلكترونياً صحيحاً",
    okRedirect: "تم تجهيز رسالتك وفتح قناة التواصل المختارة.",
  },
  en: {
    skip: "Skip to content",
    navCompare: "Compare",
    navBenefits: "Benefits",
    navSteps: "How it works",
    navCompat: "Compatibility",
    navFaq: "FAQ",
    ctaPrimary: "Request free trial",
    proofPill: "Fully offline · Compatible with Odoo 17 to 20",
    heroH1: "Silent direct printing for Odoo\nwithout an IoT Box or dialogs",
    heroSub: "Connect POS receipt, kitchen, barcode, and A4 printers directly from Windows and Android with a local WAL queue and offline licensing.",
    heroNote: "Private distribution for approved customers · No credit card needed",
    queueTitle: "Itbaa local queue · SQLite WAL",
    queueAll: "All",
    queuePrinted: "Printed",
    queueQueued: "Queued",
    statusPrinted: "Printed",
    statusQueued: "Ready",
    compareKicker: "Why Itbaa",
    compareH2: "Replace the IoT Box and monthly hardware upkeep",
    compareSub: "Compare browser printing workflows against running Itbaa directly on your existing POS and warehouse devices.",
    beforeTitle: "Browser dialogs or IoT Box",
    before1: "Buy and maintain a separate IoT Box for every branch",
    before2: "Browser print dialog interrupts every POS checkout",
    before3: "Lost print jobs when network drops or devices restart",
    before4: "Complex setup to mix ZPL barcode labels with A4 invoices",
    afterTitle: "With Itbaa on Windows and Android",
    after1: "Runs directly on existing POS PCs and Android tablets with zero extra hardware",
    after2: "Instant one click silent printing from Odoo POS and reports",
    after3: "Local SQLite WAL queue prevents duplicates and retries transient faults safely",
    after4: "Unified support for 80mm receipts, ZPL/TSPL labels, and A4 tax invoices",
    tagline: "Itbaa is built to run quietly at peak checkout, with a local queue that never drops a receipt and offline licensing that never stalls when internet drops.",
    benefitsKicker: "Practical benefits",
    benefitsH2: "Four pillars protecting checkout and warehouse speed",
    b1Title: "Ordered SQLite WAL queue",
    b1Desc: "Per job idempotency keys, per printer FIFO ordering, and safe crash recovery without blind reprints.",
    b2Title: "Zero server Ed25519 licensing",
    b2Desc: "Sub millisecond offline cryptographic verification bound to your exact Odoo origin.",
    b3Title: "Multi protocol native engine",
    b3Desc: "Direct printing over TCP 9100, IPP, Bluetooth, USB, Windows spooler, and PDFium.",
    b4Title: "Central Windows GPO control",
    b4Desc: "Lock your Odoo origin and license key via Group Policy while preserving activation across updates.",
    stepsKicker: "How it works",
    stepsH2: "Printing silently in three steps",
    s1Title: "Request your private trial",
    s1Desc: "Share your company details and Odoo version to receive your private package link and trial key.",
    s2Title: "Install module and client",
    s2Desc: "Install the Odoo addon and the Windows or Android app, then enter your Odoo URL and activation key.",
    s3Title: "Print silently from any screen",
    s3Desc: "Route receipts, kitchen tickets, labels, and A4 invoices straight to each printer with no dialogs.",
    compatKicker: "Compatibility matrix",
    compatH2: "Fits your existing Odoo stack",
    thComponent: "Component",
    thSupport: "Supported platforms",
    thDetails: "Runtime details",
    c1Name: "Odoo versions",
    c1Sup: "Odoo 17 · 18 · 19 · 20",
    c1Det: "POS receipts, PDF reports, and barcode labels",
    c2Name: "Windows desktop",
    c2Sup: "Windows 10 and Windows 11",
    c2Det: "Lightweight Rust and Tauri app with GPO policy support",
    c3Name: "Android devices",
    c3Sup: "Android 8 to Android 15",
    c3Det: "Native Kotlin app for POS terminals and tablets",
    c4Name: "Protocols and printers",
    c4Sup: "ESC/POS · ZPL · TSPL · IPP · PDF",
    c4Det: "LAN TCP 9100, USB, Bluetooth, and Windows Spooler",
    faqKicker: "Frequently asked questions",
    faqH2: "Direct answers before you start",
    f1q: "Do I need an Odoo IoT Box to use Itbaa?",
    f1a: "No. Itbaa removes the IoT Box completely and runs directly on your Windows PC or Android tablet.",
    f2q: "Does licensing still work if branch internet goes offline?",
    f2a: "Yes. Verification uses offline Ed25519 signatures locally with no external activation server calls.",
    f3q: "How do I receive installation files after requesting a trial?",
    f3a: "To protect code integrity, packages and trial keys are delivered via private links after reviewing your request.",
    f4q: "Does Itbaa support Arabic receipts and ZATCA tax invoices?",
    f4a: "Yes. Full Arabic rendering, QR codes, barcodes, and A4 tax invoices are supported via PDFium and thermal engines.",
    f5q: "What happens if a printer disconnects mid shift?",
    f5a: "The local SQLite WAL queue persists jobs and retries transient faults safely without duplicate prints.",
    f6q: "Will my license stay active after updating the desktop or Android app?",
    f6a: "Yes. Keys are stored outside the installation folder in AppData, GPO, or Android SharedPreferences.",
    f7q: "Can one branch route to multiple printers simultaneously?",
    f7a: "Yes. Route receipts to the cashier printer, tickets to the kitchen printer, and invoices to an A4 laser printer.",
    f8q: "Does it work on Odoo.sh and On Premise deployments?",
    f8a: "Yes. It works on Odoo.sh, self hosted On Premise servers, and any environment allowing custom Odoo addons.",
    reqKicker: "Get started",
    reqH2: "Request a free trial for your team",
    reqSub: "Enter your details to open a prefilled WhatsApp message or email with your Odoo environment specs.",
    reqRisk: "Full functional trial · Private package delivery · Direct onboarding support",
    lblName: "Full name",
    lblEmail: "Work email",
    lblCompany: "Company name",
    lblOdoo: "Odoo version",
    lblChannel: "Preferred channel",
    chWhatsapp: "WhatsApp",
    chEmail: "Email",
    submitBtn: "Request free trial",
    footerCopy: "Itbaa · All rights reserved",
    footerPrivacy: "Privacy policy",
    footerTerms: "Terms of service",
    errRequired: "This field is required",
    errEmail: "Enter a valid email address",
    okRedirect: "Your request message is ready and your selected channel has been opened.",
  },
};

function safeStorageGet(key) {
  try { return localStorage.getItem(key); } catch { return null; }
}
function safeStorageSet(key, val) {
  try { localStorage.setItem(key, val); } catch { /* ignore storage restrictions */ }
}

let currentLang = safeStorageGet("itbaa_lang") || SITE_CONFIG.defaultLang;
let currentTheme = safeStorageGet("itbaa_theme") || (window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light");
let currentFilter = "all";

function t(key) {
  return (I18N[currentLang] && I18N[currentLang][key]) || I18N.ar[key] || key;
}

function applyTheme(theme) {
  currentTheme = theme === "dark" ? "dark" : "light";
  document.documentElement.setAttribute("data-theme", currentTheme);
  safeStorageSet("itbaa_theme", currentTheme);
  const btn = document.getElementById("themeToggle");
  if (btn) {
    btn.setAttribute("aria-label", currentTheme === "dark" ? "Switch to light mode" : "Switch to dark mode");
    btn.innerHTML = currentTheme === "dark"
      ? '<svg width="18" height="18" viewBox="0 0 256 256" fill="none" stroke="currentColor" stroke-width="18" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><circle cx="128" cy="128" r="56"/><line x1="128" y1="36" x2="128" y2="20"/><line x1="128" y1="236" x2="128" y2="220"/><line x1="36" y1="128" x2="20" y2="128"/><line x1="236" y1="128" x2="220" y2="128"/><line x1="63" y1="63" x2="52" y2="52"/><line x1="204" y1="204" x2="193" y2="193"/><line x1="193" y1="63" x2="204" y2="52"/><line x1="52" y1="204" x2="63" y2="193"/></svg>'
      : '<svg width="18" height="18" viewBox="0 0 256 256" fill="none" stroke="currentColor" stroke-width="18" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M216.7 152.6A91.9 91.9 0 0 1 103.4 39.3a92 92 0 1 0 113.3 113.3Z"/></svg>';
  }
}

function renderTaglineWords() {
  const el = document.getElementById("taglineReveal");
  if (!el) return;
  const frag = document.createDocumentFragment();
  const words = t("tagline").split(/\s+/);
  words.forEach((w, i) => {
    if (i > 0) frag.appendChild(document.createTextNode(" "));
    const span = document.createElement("span");
    span.className = "reveal-word";
    span.textContent = w;
    frag.appendChild(span);
  });
  el.replaceChildren(frag);
  updateTaglineProgress();
}

function renderQueue() {
  const grid = document.getElementById("queueGrid");
  if (!grid) return;
  const items = QUEUE_SAMPLE.filter((j) => currentFilter === "all" || j.status === currentFilter);
  const frag = document.createDocumentFragment();
  items.forEach((job) => {
    const isPrinted = job.status === "printed";
    const card = document.createElement("article");
    card.className = "queue-card";

    const top = document.createElement("div");
    top.className = "queue-row-top";
    const idEl = document.createElement("span");
    idEl.className = "caption-3 mono";
    idEl.textContent = job.id;
    const statusEl = document.createElement("span");
    statusEl.className = isPrinted ? "badge badge-ok caption-4" : "badge badge-warn caption-4";
    statusEl.textContent = isPrinted ? t("statusPrinted") : t("statusQueued");
    top.append(idEl, statusEl);

    const typeEl = document.createElement("div");
    typeEl.className = "title-4";
    typeEl.textContent = currentLang === "ar" ? job.typeAr : job.typeEn;

    const printerEl = document.createElement("div");
    printerEl.className = "caption-5 mono";
    printerEl.style.color = "var(--text-muted)";
    printerEl.textContent = job.printer;

    const bottom = document.createElement("div");
    bottom.className = "queue-row-top";
    const protoEl = document.createElement("span");
    protoEl.className = "badge badge-brand caption-4 mono";
    protoEl.textContent = job.protocol;
    const msEl = document.createElement("span");
    msEl.className = "caption-4 mono";
    msEl.style.color = "var(--text-muted)";
    msEl.textContent = job.ms;
    bottom.append(protoEl, msEl);

    card.append(top, typeEl, printerEl, bottom);
    frag.appendChild(card);
  });
  grid.replaceChildren(frag);
}

function applyLanguage(lang) {
  currentLang = lang === "en" ? "en" : "ar";
  safeStorageSet("itbaa_lang", currentLang);
  document.documentElement.lang = currentLang;
  document.documentElement.dir = currentLang === "ar" ? "rtl" : "ltr";

  document.querySelectorAll("[data-i18n]").forEach((el) => {
    const key = el.getAttribute("data-i18n");
    const val = t(key);
    if (key === "heroH1") {
      const frag = document.createDocumentFragment();
      val.split("\n").forEach((line, idx) => {
        if (idx > 0) frag.appendChild(document.createElement("br"));
        frag.appendChild(document.createTextNode(line));
      });
      el.replaceChildren(frag);
    } else {
      el.textContent = val;
    }
  });

  const brandName = currentLang === "ar" ? SITE_CONFIG.brandAr : SITE_CONFIG.brandEn;
  document.querySelectorAll("[data-brand-name]").forEach((el) => {
    el.textContent = brandName;
  });

  const langBtn = document.getElementById("langToggle");
  if (langBtn) langBtn.textContent = currentLang === "ar" ? "EN" : "ع";

  renderTaglineWords();
  renderQueue();
}

/* B11 Word-by-word Tagline Reveal throttled through requestAnimationFrame */
let taglineRaf = 0;
function updateTaglineProgress() {
  taglineRaf = 0;
  const section = document.getElementById("tagline");
  const words = document.querySelectorAll("#taglineReveal .reveal-word");
  if (!section || !words.length) return;
  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    words.forEach((w) => w.classList.add("is-lit"));
    return;
  }
  const rect = section.getBoundingClientRect();
  const vh = window.innerHeight || 800;
  const progress = Math.max(0, Math.min(1, (vh * 0.78 - rect.top) / (vh * 0.55)));
  const litCount = Math.round(progress * words.length);
  words.forEach((w, idx) => {
    w.classList.toggle("is-lit", idx < litCount);
  });
}

function scheduleTaglineUpdate() {
  if (!taglineRaf) taglineRaf = window.requestAnimationFrame(updateTaglineProgress);
}

/* B7 IntersectionObserver Scroll Reveals (once only) */
function initScrollReveals() {
  const els = document.querySelectorAll(".reveal");
  if (!("IntersectionObserver" in window) || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    els.forEach((el) => el.classList.add("is-visible"));
    return;
  }
  const io = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        entry.target.classList.add("is-visible");
        io.unobserve(entry.target);
      }
    });
  }, { threshold: 0.12 });
  els.forEach((el) => io.observe(el));
}

/* Active Nav Section Indicator (B9) */
function initActiveNav() {
  const links = Array.from(document.querySelectorAll(".nav-links a[href^='#']"));
  const sections = links.map((a) => document.querySelector(a.getAttribute("href"))).filter(Boolean);
  if (!("IntersectionObserver" in window) || !sections.length) return;
  const io = new IntersectionObserver((entries) => {
    entries.forEach((entry) => {
      if (entry.isIntersecting) {
        const id = "#" + entry.target.id;
        links.forEach((a) => {
          if (a.getAttribute("href") === id) a.setAttribute("aria-current", "location");
          else a.removeAttribute("aria-current");
        });
      }
    });
  }, { rootMargin: "-35% 0px -55% 0px" });
  sections.forEach((s) => io.observe(s));
}

/* Validated Request Form -> WhatsApp or Email via SITE_CONFIG */
function initRequestForm() {
  const form = document.getElementById("trialForm");
  if (!form) return;
  const statusBox = document.getElementById("formStatus");

  form.addEventListener("submit", (e) => {
    e.preventDefault();
    const nameEl = document.getElementById("fName");
    const emailEl = document.getElementById("fEmail");
    const companyEl = document.getElementById("fCompany");
    const odooEl = document.getElementById("fOdoo");
    const channelEl = document.getElementById("fChannel");

    const fields = [
      { el: nameEl, errId: "errName", valid: nameEl.value.trim().length >= 2, msg: t("errRequired") },
      { el: emailEl, errId: "errEmail", valid: /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(emailEl.value.trim()), msg: t("errEmail") },
      { el: companyEl, errId: "errCompany", valid: companyEl.value.trim().length >= 2, msg: t("errRequired") },
    ];

    let allOk = true;
    fields.forEach(({ el, errId, valid, msg }) => {
      const errEl = document.getElementById(errId);
      el.setAttribute("aria-invalid", valid ? "false" : "true");
      if (errEl) errEl.textContent = valid ? "" : msg;
      if (!valid) allOk = false;
    });

    if (!allOk) {
      statusBox.className = "form-status show err body-5";
      statusBox.textContent = t("errRequired");
      return;
    }

    const name = nameEl.value.trim();
    const email = emailEl.value.trim();
    const company = companyEl.value.trim();
    const odoo = odooEl.value;
    const channel = channelEl.value;

    const bodyText = currentLang === "ar"
      ? `طلب تجربة مجانية لنظام اطبع (Itbaa)\nالاسم: ${name}\nالبريد: ${email}\nالشركة: ${company}\nإصدار أودو: ${odoo}`
      : `Itbaa Free Trial Request\nName: ${name}\nEmail: ${email}\nCompany: ${company}\nOdoo Version: ${odoo}`;

    const targetUrl = channel === "email"
      ? `mailto:${SITE_CONFIG.salesEmail}?subject=${encodeURIComponent("Itbaa Trial Request: " + company)}&body=${encodeURIComponent(bodyText)}`
      : `https://wa.me/${SITE_CONFIG.whatsappNumber}?text=${encodeURIComponent(bodyText)}`;

    statusBox.className = "form-status show ok body-5";
    statusBox.textContent = t("okRedirect");
    window.open(targetUrl, "_blank", "noopener,noreferrer");
  });
}

document.addEventListener("DOMContentLoaded", () => {
  applyTheme(currentTheme);
  applyLanguage(currentLang);
  initScrollReveals();
  initActiveNav();
  initRequestForm();

  document.getElementById("themeToggle")?.addEventListener("click", () => {
    applyTheme(currentTheme === "dark" ? "light" : "dark");
  });

  document.getElementById("langToggle")?.addEventListener("click", () => {
    applyLanguage(currentLang === "ar" ? "en" : "ar");
  });

  document.querySelectorAll(".queue-tab").forEach((btn) => {
    btn.addEventListener("click", () => {
      currentFilter = btn.getAttribute("data-filter") || "all";
      document.querySelectorAll(".queue-tab").forEach((b) => b.setAttribute("aria-pressed", b === btn ? "true" : "false"));
      renderQueue();
    });
  });

  const menuBtn = document.getElementById("menuToggle");
  const panel = document.getElementById("mobilePanel");
  if (menuBtn && panel) {
    const closeMenu = () => {
      menuBtn.setAttribute("aria-expanded", "false");
      panel.classList.remove("open");
      panel.setAttribute("aria-hidden", "true");
    };
    menuBtn.addEventListener("click", () => {
      const open = menuBtn.getAttribute("aria-expanded") !== "true";
      menuBtn.setAttribute("aria-expanded", open ? "true" : "false");
      panel.classList.toggle("open", open);
      panel.setAttribute("aria-hidden", open ? "false" : "true");
    });
    panel.querySelectorAll("a").forEach((a) => a.addEventListener("click", closeMenu));
  }

  window.addEventListener("scroll", scheduleTaglineUpdate, { passive: true });
  window.addEventListener("resize", scheduleTaglineUpdate, { passive: true });
});
