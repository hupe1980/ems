// hems household page. One document — GET /v1/home — rendered four ways.
// No framework and no build step: the box serves this file as it is written.
"use strict";

// ── Words ───────────────────────────────────────────────────────────────────
// `why` holds one sentence per explanation key the box can send, per language.
// A Rust test reads this file and fails if a key has no sentence.
const T = {
  de: {
    supply: "Stromvertrag", supplyExpired: "Preisblatt abgelaufen", supplyNewKey: "Der Lieferant verwendet einen anderen Schlüssel.", supplyFile: "Signiertes Preisblatt auswählen", supplyLocal: "Konfigurierter Tarif · kein signiertes Preisblatt", supplyNext: "Vorgemerkt", supplyCompare: "Vergleichen Sie diesen Fingerabdruck mit der Angabe Ihres Lieferanten.", supplyUse: "Vertrag übernehmen", supplyCancel: "Verwerfen", supplyFrom: "Wirksam ab", supplyPeriod: "Preisjahr", supplyEnd: "Gültig bis", supplyNoPrice: "Marktpreis für diese Viertelstunde ausstehend", supplySaved: "Vertrag vorgemerkt.", supplyLarge: "Das Preisblatt ist zu groß.", supplyEstimate: "Schätzpreis", supplyCurrent: "Aktueller Preis", supplySign: "Schlüssel", supplyUnknown: "Die Bestätigung wurde unterbrochen. Prüfen Sie den vorgemerkten Vertrag.",
    matterTitle: "Matter-Gerät", matterPoint: "Endpunkt", matterNode: "Knoten", matterTime: "Zuletzt gelesen", matter_waiting: "Erste Messung ausstehend", matter_current: "Aktuelle Wirkleistung", matter_unknown: "Messwert unbekannt", matter_stale: "Letzte Messung · veraltet", matter_unavailable: "Messung nicht verfügbar",
    "tab.now": "Jetzt", "tab.plan": "Plan", "tab.day": "Tag", "tab.setup": "Einrichtung",
    live: "Live", stale: "Keine Verbindung — letzte Werte von {t}", demo: "Demo · Referenztag", home: "Zuhause",
    sCheapRun: "Strom ist jetzt günstig (<em>{ct} ct</em>) — {dev} nutzt das.",
    signin: "Anmelden", signinHelp: "Den Zugangscode finden Sie auf dem Übergabeblatt oder im Start-Log der Box.", signinWrong: "Diesen Code hat diese Box nicht ausgegeben.", tokenLabel: "Zugangscode",
    fromGrid: "aus dem Netz", toGrid: "ins Netz", perKwh: "je kWh",
    sNoData: "Die Box hat noch nichts gemessen.",
    sReduced: "Ihr <span class=op>Netzbetreiber dimmt den Anschluss</span>. Die Box teilt die garantierte Leistung nach Bedarf auf.",
    sReducedCovered: "Ihr <span class=op>Netzbetreiber dimmt den Anschluss</span>. Der Speicher trägt das Haus — Sie merken nichts davon.",
    sFailsafe: "Die Steuerbox meldet sich nicht. Bis sie es wieder tut, gilt der hinterlegte <span class=op>Failsafe</span>-Wert.",
    sFeed: "Ihr Dach speist gerade <em>{kw} kW</em> ein.",
    sCharging: "{dev} lädt jetzt — <em>{ct} ct</em> gehört zu den günstigsten Stunden der nächsten zwei Tage.",
    sCovering: "Der Speicher deckt das Haus, damit Sie zu <em>{ct} ct</em> fast nichts kaufen.",
    sPlain: "Das Haus nimmt, was es braucht. Alles Flexible wartet auf eine günstigere Zeit.",
    chipCeiling: "§ 14a · Begrenzung auf {kw} kW", chipReduced: "§ 14a · {kw} kW garantiert", chipFailsafe: "Failsafe · {kw} kW",
    skipContent: "Zum Inhalt", viewsLabel: "Ansichten", replayTime: "Uhrzeit im Referenztag", replayPlay: "Wiedergabe starten", replayPause: "Wiedergabe anhalten",
    shared: "So teilt die Box {kw} kW auf", sharedHelp: "Nach Bedarf. § 14a begrenzt den Bezug aus dem Netz — was Dach und Speicher decken, kommt hinzu. Ein Gerät, das nur misst, bekommt nichts zugeteilt.",
    cheapNow: "Jetzt günstig", cheapUntil: "bis {t}", cheapNext: "Günstig ab {t}", cheapFor: "für {h} h", cheapNone: "Kein günstiges Fenster im Plan",
    car: "Auto", heat_pump: "Wärmepumpe", battery: "Speicher", hot_water: "Warmwasser", appliance: "Gerät", roof: "Dach",
    carCharging: "lädt mit {kw} kW", carIdle: "angesteckt, wartet", carGoal: "{pct} % bis {t}", carNoGoal: "Ziel setzen",
    resting: "Ruhen", devices: "Geräte", next24: "Die nächsten 24 Stunden",
    historyRepairTitle: "Messhistorie", history_repair_pending: "Gespeicherte Viertelstunden warten auf eine Neuberechnung. Nach der automatischen Wartung erneut prüfen.", history_busy: "Die Messhistorie wird gerade gelesen. Anschließend erneut prüfen.", history_unavailable: "Status der Messhistorie nicht verfügbar. Gateway-Protokoll prüfen.",
    healthTitle: "Anlagenstatus", healthCount: "{n} von {total} Geräten antworten", healthCheck: "Verbindung und Stromversorgung am Gerät prüfen.", healthResponse: "Sollwert und Messwert am Gerät vergleichen.", healthUnknown: "Noch kein Messwert",
    notice_silent: "Meldet sich nicht", notice_assumed: "Wert angenommen", notice_disobedient: "Folgt nicht", notice_undriven: "Nicht angebunden",
    planTitle: "Die nächsten zwei Tage", legendCheap: "günstig", legendDraw: "Bezug", legendOp: "Netzbetreiber", reducedSlot: "§ 14a-Dimmung",
    nothing: "Nichts geplant", nothingWhy: "Das Haus nimmt, was es braucht.",
    contrast: "Dieselbe Energie im günstigsten Fenster ({best} ct) kostete {diff} weniger.", contrastBest: "Das ist das günstigste Fenster im Plan.",
    goal: "Ziel fürs Auto", goalTank: "Ziel fürs Warmwasser", goalTo: "Laden auf", goalHeat: "Heizen auf", goalBy: "Bis", goalClear: "Ziel löschen", tankGoal: "{c} °C bis {t}",
    previousPlan: "Letzte Planung fehlgeschlagen. Angezeigt wird der Plan vom {time}.",
    worthStore: "Eine Kilowattstunde im Speicher ist Ihnen jetzt {b} ct wert; eingespeist bringt sie {e} ct — darum geht der Überschuss in den Speicher.",
    worthSell: "Eingespeist bringt eine Kilowattstunde jetzt {e} ct; im Speicher wäre sie Ihnen {b} ct wert — darum wird eingespeist.",
    runCar: "Das Auto lädt {a}–{b}.", runBatteryIn: "Der Speicher lädt {a}–{b}.", runBatteryOut: "Der Speicher versorgt das Haus {a}–{b}.", runHot: "Das Warmwasser heizt {a}–{b}.", runHeat: "Die Wärmepumpe läuft {a}–{b}.",
    qBoostReview: "Sofortladen prüfen", qBoost: "Jetzt laden", qStopBoost: "Sofortladen beenden", qBoostCheck: "Ladestatus prüfen",
    qCap: "Preislimit (optional)", qEligible: "{n} Viertelstunden bis {cap} ct/kWh",
    qAbout: "Ungefähr {price}",
    qMore: "{eur} mehr als jetzt", qLess: "{eur} weniger als jetzt", qFree: "Keine Mehrkosten", qSet: "Übernehmen", qAsk: "Wird berechnet …", qNoCost: "Kosten unbekannt", qSaved: "Gespeichertes Ziel", qNone: "Kein gespeichertes Ziel", qCancel: "Entwurf verwerfen", qChecking: "Gespeichertes Ziel wird geprüft …", qRetry: "Preis erneut prüfen", qBusy: "Eine Berechnung läuft. Bitte gleich erneut versuchen.", qChanged: "Das Angebot hat sich geändert. Bitte erneut prüfen.",
    aheadBill: "Bis {t} voraussichtlich <b>{eur}</b> Strom", aheadEarn: "Bis {t} voraussichtlich <b>{eur}</b> Erlös",
    v_met: "erreicht", v_feasible: "machbar", v_tight: "knapp", v_impossible: "nicht machbar",
    vt_met: "Schon erreicht.", vt_feasible: "{kwh} kWh in den günstigsten Stunden.", vt_tight: "Nur, wenn nichts anderes den Anschluss braucht.", vt_impossible: "Bis dahin sind {reach} möglich.",
    receipt: "Kassenbon", bill: "Stromrechnung", wear: "Speicherverschleiß", comfort: "Komfortverzicht", stores: "Speicherstand-Ausgleich", cost: "Kosten des Tages",
    without: "Gleiches Haus, gleicher Speicher, ohne hems", saved: "Gespart", onBill: "…davon auf der Rechnung",
    bought: "Gekauft", made: "Vom Dach", reductionMin: "§ 14a-Dimmung", respected: "Dimmung eingehalten", feedRespected: "Einspeisegrenze eingehalten", yes: "ja", no: "nein",
    method: "So wird gerechnet", methodText: "Derselbe Tag läuft zweimal im selben Haus mit demselben Speicher: einmal mit Plan, einmal mit einer Eigenverbrauchsregelung. Die Differenz ist die Ersparnis. Endet ein Tag mit leererem Speicher, zählt das Nachladen mit. Verschleiß und Komfort haben einen Preis, damit eine mit kaltem Zimmer oder müdem Akku erkaufte Ersparnis nicht zählt.",
    gridHistory: "Netzanschluss · Viertelstunde prüfen", historyDate: "Tag in Berlin", historyQuarter: "Viertelstunde ab", historyAverage: "Mittlere Leistung", historyMin: "Minimum", historyMax: "Maximum", historySamples: "Aufgezeichnete Messwerte", historyUnknown: "Nicht verfügbar", historySign: "Positiv bedeutet Bezug, negativ Einspeisung.", historyLoading: "Messwerte werden geladen …", historyEmpty: "Für diese Viertelstunde liegt kein Messwert vor.", historyOff: "Die lokale Messwerthistorie ist nicht eingerichtet.", historyFailure: "Die Messwerte konnten nicht geladen werden.", historyBusy: "Die Box bearbeitet gerade eine andere Historienabfrage.", historyRetry: "Erneut versuchen",
    export: "Lokale Leistungshistorie laden (JSON)", exportBusy: "Historie wird zusammengestellt …", exportDone: "Download bereit.", exportPartial: "Teildownload · Diese Messpunkte fehlen: {points}.", exportFailed: "Keine Datei erstellt. Messpunkte nicht abrufbar: {points}.", noReceipt: "Der Kassenbon entsteht um Mitternacht.",
    forOperator: "Für Ihren Messstellenbetreiber", ski: "SKI", fp: "Zertifikat (SHA-256)", fpNote: "Bleibt nach Neustart gleich.",
    steuerbox: "Steuerbox", waiting: "Möchte sich koppeln — SKI mit dem Etikett vergleichen.", approve: "Koppeln", refuse: "Ablehnen",
    trusted: "Gekoppelt", noPeer: "Noch keine Steuerbox gekoppelt.", handover: "Übergabeblatt drucken", signout: "Abmelden", language: "Sprache",
    approved: "Gekoppelt. Die Box folgt ab jetzt dieser Steuerbox.", refused: "Abgelehnt.",
    leviesYear: "Die Umlagen für dieses Jahr sind noch nicht veröffentlicht — gerechnet wird mit denen von {y}.",
    transferMeter: "Die aktuelle Messung am Netzanschluss fehlt.", transferActuator: "Die Steuerfähigkeit aller Geräte muss vor der Übergabe nachgewiesen sein.",
    authorizeTransfer: "Übergabe freigeben", revokeTransfer: "Freigabe zurücknehmen", transferHelp: "Bis zur Übergabe durch den Installateur steuert {who}. Die Freigabe gilt eine Stunde.", transferConfirm: "Den Installateur für eine Stunde zur Übergabe an diese Box berechtigen?",
    measuredMethod: "Die Box hält den gemessenen Hausverbrauch in beiden Läufen gleich. Die Modellabweichung summiert die absoluten Kostendifferenzen zum Zähler je Viertelstunde. Nur Unterschiede oberhalb dieser Abweichung zählen.",
    boughtBlock: "Netzladung", servedBlock: "Aus dem Speicher versorgt", blockMargin: "Marge nach Speicherverlusten", blockCount: "{n} gemessene Ladezyklen",
    planModel: "Mit Plan (Modell)", idleModel: "Eigenverbrauch (Modell)", replayErr: "Abweichung vom Zähler", similarBill: "Innerhalb der Modellabweichung", netDifference: "Kostendifferenz", matchedDays: "Aussagekräftige Vergleichstage · {n}", noComparison: "Für den Vergleich fehlen durchgehende Messwerte für den gleichen Betrieb.",
    shadowMode: "Beobachtung", shadowReads: "Die Box misst. Ein anderer Regler steuert.", watchedIntervals: "{n} Viertelstunden im Beobachtungsmodus", shadowPlan: "So würde die Box planen",
    monthlyBill: "Stromkosten für {n} vollständig abgerechnete Tage",
    measuredCost: "Bezugsstrom abzüglich Einspeisung", fixedCharges: "Anteil der jährlichen Fixkosten", pricedCoverage: "Abrechnung unvollständig · {n} von {total} Viertelstunden mit Messwert und Preis",
    learningDays: "Prognose lernt · {n} von 14 Tagen ausgewertet", forecastCoverage: "Messwerte im Prognoseband", forecastBands: "Dach {pv} % · Hauslast {load} %",
    month: "Monat", savedOver: "gespart an {n} Tagen", monthCost: "Kosten {c} · ohne hems {w}", csv: "Tage als CSV laden", latestDay: "Zum letzten Tag", earlier: "Früher", later: "Später",
    sheet: "Übergabeblatt", sheetFor: "Für Messstellenbetreiber, Netzbetreiber und Haushalt", shipId: "SHIP-ID", print: "Drucken", sca: "Steuerbare Verbrauchseinrichtungen · § 14a EnWG",
    fg_ladepunkt: "Ladepunkt", fg_waermepumpe: "Wärmepumpe", fg_raumkuehlung: "Raumkühlung", fg_stromspeicher: "Stromspeicher", thDevice: "Einrichtung", thPower: "Leistung", thMin: "Mindestens", thSince: "In Betrieb",
    sheetSum: "Steuerung über das Energiemanagementsystem · garantierte Mindestleistung {min} kW{mod}", sheetModule: " · Modul {m}",
    sheetSignin: "Anmelden", sheetSigninHelp: "Mit dem Telefon scannen, dann zum Home-Bildschirm hinzufügen. Dieses Blatt ist ein Schlüssel — bitte so aufbewahren.",
    record: "Abschluss-Code", recordHelp: "Für das Auftragssystem des Installateurs: die Box bestätigt mit ihrer Signatur, was gekoppelt und eingerichtet ist. Auftragsnummer eintragen, dann scannen.", orderNo: "Auftragsnummer", recordFile: "Als Datei laden", recordOpen: "Abschluss-Code für den Auftrag", recordNone: "Ohne Auftragsnummer nennt der Code keinen Auftrag.", recordBig: "Für einen Code ist dieser Nachweis zu groß — bitte die Datei laden.", householdOnly: "Nur für den Haushalt — nicht fotografieren",
    needCode: "Für den Anmelde-Code den Zugangscode noch einmal eingeben.", ok: "OK",
    newCode: "Neuen Zugangscode ausgeben", newCodeHelp: "Macht das alte Übergabeblatt und alle Anmeldungen ungültig.", newCodeDone: "Neuer Zugangscode — jetzt das Übergabeblatt neu drucken.",
    heartbeatAge: "Letzter Heartbeat vor {s} s", trust_heard: "Die Steuerbox ist gekoppelt und meldet sich.", trust_silent: "Die Steuerbox ist gekoppelt, schweigt aber.", trust_failsafe: "Die Steuerbox schweigt — der Failsafe-Wert gilt.", trust_unpaired: "Eine Steuerbox ist vorgesehen, aber nicht gekoppelt.",
    id_kept: "Die Box hat dieselbe Identität wie vor dem Neustart.", id_ephemeral: "Die Box behält ihre Identität nicht — nach einem Neustart muss neu gekoppelt werden.",
    find: "Geräte im Netz", findHosts: "Adressen, z. B. 192.168.1.40", findGo: "Suchen", findNone: "Nichts gefunden.",
    meter_restarting: "Gateway startet neu und wartet auf die gewählten Geräte.", meter_applied: "Einrichtung abgeschlossen · alle gewählten Geräte melden Messwerte.", meter_restored: "Vorherige Konfiguration wiederhergestellt. Gewählte Geräte und Modbus-Units prüfen.",
    applyPv: "Übernehmen und Gateway neu starten", applyConfirm: "Diesen Wechselrichter messen und begrenzen? Das Gateway startet neu.", change_restarting: "Gateway startet neu und wartet auf den Wechselrichter.", change_applied: "Wechselrichter eingerichtet · Messung empfangen.", change_restored: "Vorherige Konfiguration wiederhergestellt. Verbindung zum Wechselrichter prüfen.", change_superseded: "Die Konfiguration wurde außerhalb der Einrichtung geändert.", change_unknown: "Verbindung unterbrochen. Das Ergebnis wird beim Wiederverbinden angezeigt.",
    deviceClaim: "Geräteangabe", reviewPv: "PV-Wechselrichter prüfen", reviewRole: "PV-Leistung messen und begrenzen", reviewOk: "Konfiguration gültig", reviewCancel: "Prüfung schließen", reviewUnsupported: "Für diese Einrichtung werden ein unterstütztes PV-Modell und SunSpec 123 benötigt.", unitLabel: "Modbus-Unit",
    st_configured: "eingerichtet", st_missing: "eingerichtet, antwortet nicht", st_found: "gefunden, nicht eingerichtet", st_silent: "keine Antwort", st_waiting: "möchte sich koppeln",
    hooks: "Andere Systeme", hooksHelp: "Home Assistant, Node-RED oder Ihr Installateur erfahren, was die Box Ihnen mitteilt — mit den Zahlen, signiert. Nur an Adressen, die Sie hier eintragen.", hookName: "Name", hookUrl: "Adresse (Webhook-URL)", hookWhat: "Was senden", hookGrid: "Dimmung und Failsafe", hookAll: "Alles", hookPublic: "Die Adresse liegt im Internet — die Ereignisse verlassen Ihr Netz", hookAdd: "Hinzufügen", hookTest: "Testen", hookRemove: "Entfernen", hookSecret: "Das Geheimnis für den Empfänger — es wird nur jetzt angezeigt:", hookCopy: "Kopieren", hookCopied: "Kopiert", hookOk: "Empfangen ({ms} ms)", hookOwed: "{n} noch in Zustellung", hookNone: "Noch kein System.",
    notify: "Mitteilungen", notifyOn: "Mitteilungen einschalten", notifyHelp: "Die Box meldet sich bei vier Anlässen — und sonst nie. Der Mitteilungsdienst Ihres Telefons sieht, wann die Box etwas sendet, aber nicht was.", notifyHome: "Auf dem iPhone die Seite zuerst zum Home-Bildschirm hinzufügen, dann hier einschalten.",
    ev_reduction: "Netzbetreiber dimmt", evd_reduction: "Beginn und Ende", ev_goal: "Ziel in Gefahr", evd_goal: "Knapp oder nicht machbar", ev_device: "Gerät folgt nicht", evd_device: "Länger als zehn Minuten", ev_failsafe: "Failsafe", evd_failsafe: "Die Steuerbox schweigt",
    quiet: "Nachtruhe", quietHelp: "22 bis 7 Uhr still", pushTest: "Probe senden", pushOff: "Ausschalten", pushDenied: "Mitteilungen sind für diese Seite gesperrt.", pushSent: "Unterwegs.",
    glance: "Wandanzeige", open: "Öffnen", glanceHelp: "Eine Ansicht für ein Tablet an der Wand. Antippen führt zurück.",
    roof_healthy: "Das Dach liefert wie gewohnt.", roof_degraded: "Das Dach liefert weniger als früher.", roof_learning: "Das Dach wird noch gelernt.",
    why: {
      "guard.lpc": "Gedimmt: Ihr Netzbetreiber begrenzt den Anschluss.",
      "guard.lpp": "Einspeisung begrenzt: Ihr Netzbetreiber hat eine Grenze gesetzt.",
      "guard.para9_cap": "Einspeisung auf 60 % begrenzt, bis ein intelligentes Messsystem steuert.",
      "guard.failsafe": "Failsafe: Die Steuerbox schweigt, der hinterlegte Wert gilt.",
      "guard.circuit_limit": "Gebremst, damit die Sicherung hält.",
      "guard.contract_limit": "Gebremst auf die vereinbarte Anschlussleistung.",
      "guard.unbalance": "Gebremst, damit die Phasen im Gleichgewicht bleiben.",
      "guard.device_limit": "Am Limit des Geräts.",
      "guard.backup_reserve": "Hält Ihre Notstromreserve.",
      "user.boost": "Sie haben Boost gedrückt.",
      "user.pause": "Sie haben pausiert.",
      "user.price_cap": "Wartet auf einen bestätigten Strompreis bis {cap} ct/kWh.",
      "user.away": "Abwesend: nur das Nötigste.",
      "plan.charge": "Läuft mit {kw} kW — {price_ct} ct gehört zu den günstigsten {among} % der nächsten zwei Tage.",
      "plan.run": "Läuft jetzt, weil der Plan die Energie hier braucht.",
      "plan.discharge": "Liefert {kw} kW, damit Sie zu {price_ct} ct nichts kaufen.",
      "plan.wait": "Wartet auf eine günstigere Zeit.",
      "plan.wait_until": "Wartet bis {at} — dann {then_ct} ct.",
      "manager": "Gesteuert von Ihrem Energiemanager.",
      "live.surplus_tracking": "Folgt dem Sonnenüberschuss.",
      "live.self_consumption": "Deckt das Haus aus dem Speicher.",
      "live.phase_balance": "Hält die Phasen im Gleichgewicht.",
      "live.ramp_limit": "Fährt sanft hoch.",
      "live.hysteresis": "Hält den Wert — die Änderung lohnt nicht.",
      "live.local_control": "Regelt sich gerade selbst.",
      "live.maximum_power_point": "Holt aus dem Dach, was es hergibt.",
      "fallback.no_plan": "Noch kein Plan — die Box bleibt vorsichtig.",
      "fallback.plan_stale": "Der Plan ist veraltet — die Box bleibt vorsichtig.",
      "fallback.driver_lost": "Keine Verbindung — die Box nimmt das Sichere an.",
      "fallback.clock_unsynchronised": "Uhr nicht synchron: Tarife pausieren, Netzregeln gelten.",
      "idle": "Aus."
    }
  },
  en: {
    supply: "Electricity contract", supplyExpired: "Price sheet expired", supplyNewKey: "The signing key has changed.", supplyFile: "Choose signed price sheet", supplyLocal: "Configured tariff · no signed price sheet", supplyNext: "Scheduled", supplyCompare: "Compare this fingerprint with the one your supplier provided.", supplyUse: "Accept contract", supplyCancel: "Discard", supplyFrom: "Effective from", supplyPeriod: "Priced year", supplyEnd: "Valid through", supplyNoPrice: "Market price pending for this quarter hour", supplySaved: "Contract scheduled.", supplyLarge: "The price sheet is too large.", supplyEstimate: "Estimated price", supplyCurrent: "Current price", supplySign: "Signing key", supplyUnknown: "Confirmation was interrupted. Check the scheduled contract.",
    matterTitle: "Matter appliance", matterPoint: "Endpoint", matterNode: "Node", matterTime: "Last read", matter_waiting: "Awaiting first reading", matter_current: "Current active power", matter_unknown: "Measurement unknown", matter_stale: "Last reading · stale", matter_unavailable: "Measurement unavailable",
    "tab.now": "Now", "tab.plan": "Plan", "tab.day": "Day", "tab.setup": "Setup",
    live: "Live", stale: "No contact — last values from {t}", demo: "Demo · reference day", home: "Home",
    sCheapRun: "Power is cheap right now (<em>{ct} ct</em>) — the {dev} is using it.",
    signin: "Sign in", signinHelp: "The access code is on the handover sheet or in the box's start-up log.", signinWrong: "That is not a code this box issued.", tokenLabel: "Access code",
    fromGrid: "from the grid", toGrid: "to the grid", perKwh: "per kWh",
    sNoData: "The box has not measured anything yet.",
    sReduced: "Your <span class=op>network operator is reducing</span> the connection. The box shares the guaranteed power by need.",
    sReducedCovered: "Your <span class=op>network operator is reducing</span> the connection. The battery is carrying the house, so you notice nothing.",
    sFailsafe: "The control box is silent. Until it is heard from again, the stored <span class=op>failsafe</span> value applies.",
    sFeed: "Your roof is feeding in <em>{kw} kW</em>.",
    sCharging: "The {dev} is charging now — <em>{ct} ct</em> is among the cheapest hours of the next two days.",
    sCovering: "The battery is covering the house, so you buy almost nothing at <em>{ct} ct</em>.",
    sPlain: "The house is taking what it needs. Everything flexible waits for a cheaper time.",
    chipCeiling: "§ 14a · limited to {kw} kW", chipReduced: "§ 14a · {kw} kW guaranteed", chipFailsafe: "Failsafe · {kw} kW",
    skipContent: "Skip to content", viewsLabel: "Views", replayTime: "Reference day time", replayPlay: "Play replay", replayPause: "Pause replay",
    shared: "How the box shares {kw} kW", sharedHelp: "By need. § 14a limits what comes from the grid — what the roof and the battery cover is added to it. A device that only measures is given no share.",
    cheapNow: "Cheap now", cheapUntil: "until {t}", cheapNext: "Cheap from {t}", cheapFor: "for {h} h", cheapNone: "No cheap window in the plan",
    car: "Car", heat_pump: "Heat pump", battery: "Battery", hot_water: "Hot water", appliance: "Appliance", roof: "Roof",
    carCharging: "charging at {kw} kW", carIdle: "plugged in, waiting", carGoal: "{pct} % by {t}", carNoGoal: "Set a goal",
    resting: "Resting", devices: "Devices", next24: "The next 24 hours",
    historyRepairTitle: "Meter history", history_repair_pending: "Stored quarters await recalculation. Check again after automatic maintenance.", history_busy: "Another request is reading history. Retry when it finishes.", history_unavailable: "History status is unavailable. Check the gateway log.",
    healthTitle: "Device health", healthCount: "{n} of {total} devices responding", healthCheck: "Check the device connection and power supply.", healthResponse: "Compare the request with the reading at the device.", healthUnknown: "Awaiting a first measurement",
    notice_silent: "Not reporting", notice_assumed: "Value assumed", notice_disobedient: "Not following", notice_undriven: "Not connected",
    planTitle: "The next two days", legendCheap: "cheap", legendDraw: "draw", legendOp: "network operator", reducedSlot: "§ 14a reduction",
    nothing: "Nothing scheduled", nothingWhy: "The house takes what it needs.",
    contrast: "The same energy in the cheapest window ({best} ct) would cost {diff} less.", contrastBest: "This is the cheapest window in the plan.",
    goal: "Goal for the car", goalTank: "Goal for hot water", goalTo: "Charge to", goalHeat: "Heat to", goalBy: "By", goalClear: "Clear goal", tankGoal: "{c} °C by {t}",
    previousPlan: "The latest planning attempt failed. Showing the plan from {time}.",
    worthStore: "A kilowatt-hour in the battery is worth {b} ct to you now; exported it earns {e} ct — so the surplus goes into the battery.",
    worthSell: "Exported, a kilowatt-hour earns {e} ct now; in the battery it would be worth {b} ct to you — so it is exported.",
    runCar: "The car charges {a}–{b}.", runBatteryIn: "The battery charges {a}–{b}.", runBatteryOut: "The battery carries the house {a}–{b}.", runHot: "Hot water heats {a}–{b}.", runHeat: "The heat pump runs {a}–{b}.",
    qBoostReview: "Preview charging now", qBoost: "Charge now", qStopBoost: "End immediate charging", qBoostCheck: "Check charging status",
    qCap: "Price cap (optional)", qEligible: "{n} quarters at or below {cap} ct/kWh",
    qAbout: "About {price}",
    qMore: "{eur} more than now", qLess: "{eur} less than now", qFree: "No extra cost", qSet: "Set", qAsk: "Working it out …", qNoCost: "Cost unavailable", qSaved: "Saved goal", qNone: "No saved goal", qCancel: "Discard draft", qChecking: "Checking saved goal …", qRetry: "Retry quote", qBusy: "A quote is already running. Try again in a moment.", qChanged: "Quote changed. Review the new estimate.",
    aheadBill: "About <b>{eur}</b> of electricity until {t}", aheadEarn: "About <b>{eur}</b> earned until {t}",
    v_met: "met", v_feasible: "feasible", v_tight: "tight", v_impossible: "impossible",
    vt_met: "Already there.", vt_feasible: "{kwh} kWh in the cheapest hours.", vt_tight: "Only if nothing else needs the connection.", vt_impossible: "By then {reach} is possible.",
    receipt: "Receipt", bill: "Electricity bill", wear: "Battery wear", comfort: "Comfort given up", stores: "Store level adjustment", cost: "Cost of the day",
    without: "Same house, same battery, no hems", saved: "Saved", onBill: "…of it on the bill",
    bought: "Bought", made: "From the roof", reductionMin: "§ 14a reduction", respected: "Reduction respected", feedRespected: "Feed-in ceiling respected", yes: "yes", no: "no",
    method: "How the saving is worked out", methodText: "The same day runs twice on the same house with the same battery: once with the plan, once with a self-consumption controller. The difference is the saving. A day that ends with emptier stores is charged for refilling them. Wear and comfort are priced, so a saving bought with a cold room or a tired battery does not count.",
    gridHistory: "Grid connection · inspect a quarter hour", historyDate: "Date in Berlin", historyQuarter: "Quarter hour starting", historyAverage: "Mean power", historyMin: "Minimum", historyMax: "Maximum", historySamples: "Samples recorded", historyUnknown: "Unavailable", historySign: "Positive means import, negative means export.", historyLoading: "Loading readings …", historyEmpty: "No reading was recorded for this quarter hour.", historyOff: "Local measurement history is not configured.", historyFailure: "The readings could not be loaded.", historyBusy: "The gateway is handling another history query.", historyRetry: "Try again",
    export: "Download local power history (JSON)", exportBusy: "Preparing the download …", exportDone: "Download ready.", exportPartial: "Partial download · Missing points: {points}.", exportFailed: "No file created. Could not load: {points}.", noReceipt: "The receipt is printed at midnight.",
    forOperator: "For your metering point operator", ski: "SKI", fp: "Certificate (SHA-256)", fpNote: "Stays the same after a restart.",
    steuerbox: "Control box", waiting: "Asking to pair — compare the SKI with its label.", approve: "Pair", refuse: "Refuse",
    trusted: "Paired", noPeer: "No control box paired yet.", handover: "Print the handover sheet", signout: "Sign out", language: "Language",
    approved: "Paired. The box follows this control box from now on.", refused: "Refused.",
    leviesYear: "This year's levies are not published yet — the box is pricing on those of {y}.",
    transferMeter: "Waiting for a current reading from the connection meter.", transferActuator: "The gateway cannot verify control of every actuator on this site.",
    authorizeTransfer: "Authorize handover", revokeTransfer: "Revoke authorization", transferHelp: "{who} remains in control until the installer completes the transfer. Authorization lasts one hour.", transferConfirm: "Authorize your installer to transfer control to this gateway within one hour?",
    measuredMethod: "Both runs use the same measured household demand. Model error adds up the absolute cost differences against the meter each quarter hour. Only differences above this error count.",
    boughtBlock: "Grid charging", servedBlock: "Served by storage", blockMargin: "Margin after storage losses", blockCount: "{n} measured cycles",
    planModel: "With plan (model)", idleModel: "Self-consumption (model)", replayErr: "Model error against meter", similarBill: "Within model error", netDifference: "Cost difference", matchedDays: "Conclusive comparison days · {n}", noComparison: "A comparison needs complete readings for equivalent service.",
    shadowMode: "Observing", shadowReads: "This gateway measures. A separate controller operates the site.", watchedIntervals: "{n} intervals in observation mode", shadowPlan: "What the gateway would plan",
    monthlyBill: "Electricity cost for {n} fully priced days",
    measuredCost: "Imported electricity less export revenue", fixedCharges: "Share of annual fixed charges", pricedCoverage: "Partial bill · {n} of {total} intervals with readings and prices",
    learningDays: "Forecast learning · {n} of 14 days scored", forecastCoverage: "Readings within the forecast range", forecastBands: "Solar {pv}% · household load {load}%",
    month: "Month", savedOver: "saved over {n} days", monthCost: "Cost {c} · without hems {w}", csv: "Download the days as CSV", latestDay: "Back to the latest day", earlier: "Earlier", later: "Later",
    sheet: "Handover sheet", sheetFor: "For the metering point operator, the network operator and the household", shipId: "SHIP ID", print: "Print", sca: "Controllable devices · § 14a EnWG",
    fg_ladepunkt: "Charge point", fg_waermepumpe: "Heat pump", fg_raumkuehlung: "Room cooling", fg_stromspeicher: "Battery", thDevice: "Device", thPower: "Power", thMin: "At least", thSince: "In service",
    sheetSum: "Controlled through the energy management system · guaranteed minimum {min} kW{mod}", sheetModule: " · module {m}",
    sheetSignin: "Sign in", sheetSigninHelp: "Scan with a phone, then add the page to the home screen. This sheet is a key — keep it like one.",
    record: "Completion code", recordHelp: "For the installer's order system: the box confirms, under its signature, what is paired and set up. Enter the order number, then scan.", orderNo: "Order number", recordFile: "Download as a file", recordOpen: "Completion code for the order", recordNone: "Without an order number the code names no order.", recordBig: "This record is too large for a code — download the file.", householdOnly: "For the household only — do not photograph",
    needCode: "Enter the access code once more to print the sign-in code.", ok: "OK",
    newCode: "Issue a new access code", newCodeHelp: "Makes the old handover sheet and every sign-in invalid.", newCodeDone: "New access code — print the handover sheet again now.",
    heartbeatAge: "Last heartbeat {s} s ago", trust_heard: "The control box is paired and reporting.", trust_silent: "The control box is paired but silent.", trust_failsafe: "The control box is silent — the failsafe value applies.", trust_unpaired: "A control box is expected but not paired.",
    id_kept: "The box has the same identity as before the restart.", id_ephemeral: "The box does not keep its identity — it must be paired again after a restart.",
    find: "Devices on the network", findHosts: "Addresses, e.g. 192.168.1.40", findGo: "Search", findNone: "Nothing found.",
    meter_restarting: "Gateway restarting · waiting for the selected devices.", meter_applied: "Setup complete · all selected devices are reporting.", meter_restored: "Previous configuration restored. Check the selected devices and Modbus units.",
    applyPv: "Apply and restart gateway", applyConfirm: "Measure and limit this inverter? The gateway will restart.", change_restarting: "Gateway restarting · waiting for the inverter.", change_applied: "Inverter configured · measurement received.", change_restored: "Previous configuration restored. Check the inverter connection.", change_superseded: "The configuration was changed outside Setup.", change_unknown: "Connection interrupted. The outcome will appear when the gateway reconnects.",
    deviceClaim: "Reported by device", reviewPv: "Review PV inverter", reviewRole: "Measure and limit PV output", reviewOk: "Configuration valid", reviewCancel: "Close review", reviewUnsupported: "Setup requires a supported PV inverter model and SunSpec 123.", unitLabel: "Modbus unit",
    st_configured: "configured", st_missing: "configured, not answering", st_found: "found, not configured", st_silent: "no answer", st_waiting: "asking to pair",
    hooks: "Other systems", hooksHelp: "Home Assistant, Node-RED or your installer hear what the box tells you — with the numbers, signed. Only at addresses you enter here.", hookName: "Name", hookUrl: "Address (webhook URL)", hookWhat: "What to send", hookGrid: "Reductions and failsafe", hookAll: "Everything", hookPublic: "This address is on the internet — the events leave your network", hookAdd: "Add", hookTest: "Test", hookRemove: "Remove", hookSecret: "The secret for the receiver — shown only now:", hookCopy: "Copy", hookCopied: "Copied", hookOk: "Received ({ms} ms)", hookOwed: "{n} still being delivered", hookNone: "No system yet.",
    notify: "Notifications", notifyOn: "Turn on notifications", notifyHelp: "The box speaks up on four occasions — and never otherwise. Your phone's notification service sees when the box sends something, not what.", notifyHome: "On an iPhone, add the page to the home screen first, then turn them on here.",
    ev_reduction: "Operator reduces", evd_reduction: "When it begins and ends", ev_goal: "Goal at risk", evd_goal: "Tight or impossible", ev_device: "Device not following", evd_device: "For more than ten minutes", ev_failsafe: "Failsafe", evd_failsafe: "The control box is silent",
    quiet: "Quiet hours", quietHelp: "Silent from 22:00 to 07:00", pushTest: "Send a test", pushOff: "Turn off", pushDenied: "Notifications are blocked for this page.", pushSent: "On its way.",
    glance: "Wall display", open: "Open", glanceHelp: "One view for a tablet on the wall. Tap it to come back.",
    roof_healthy: "The roof is delivering as usual.", roof_degraded: "The roof is delivering less than it used to.", roof_learning: "Still learning the roof.",
    why: {
      "guard.lpc": "Reduced: your network operator is limiting the connection.",
      "guard.lpp": "Feed-in limited: your network operator has set a ceiling.",
      "guard.para9_cap": "Feed-in capped at 60 % until a smart meter can control it.",
      "guard.failsafe": "Failsafe: the control box is silent, so the stored value applies.",
      "guard.circuit_limit": "Held back so the fuse holds.",
      "guard.contract_limit": "Held to the agreed connection power.",
      "guard.unbalance": "Held back to keep the phases balanced.",
      "guard.device_limit": "At the device's own limit.",
      "guard.backup_reserve": "Keeping your backup reserve.",
      "user.boost": "You pressed boost.",
      "user.pause": "You paused it.",
      "user.price_cap": "Waiting for a confirmed price at or below {cap} ct/kWh.",
      "user.away": "Away: only what is needed.",
      "plan.charge": "Running at {kw} kW — {price_ct} ct is among the cheapest {among} % of the next two days.",
      "plan.run": "Running now because the plan needs the energy here.",
      "plan.discharge": "Delivering {kw} kW so you buy nothing at {price_ct} ct.",
      "plan.wait": "Waiting for a cheaper time.",
      "plan.wait_until": "Waiting until {at} — {then_ct} ct then.",
      "manager": "Driven by your energy manager.",
      "live.surplus_tracking": "Following the solar surplus.",
      "live.self_consumption": "Covering the house from the battery.",
      "live.phase_balance": "Keeping the phases balanced.",
      "live.ramp_limit": "Ramping up gently.",
      "live.hysteresis": "Holding steady — the change is not worth it.",
      "live.local_control": "Regulating itself right now.",
      "live.maximum_power_point": "Taking everything the roof gives.",
      "fallback.no_plan": "No plan yet, so the box stays cautious.",
      "fallback.plan_stale": "The plan is out of date, so the box stays cautious.",
      "fallback.driver_lost": "Lost contact, so the box assumes the safe value.",
      "fallback.clock_unsynchronised": "Clock not synchronised: tariffs pause, grid rules still apply.",
      "idle": "Off."
    }
  }
};

// ── State ───────────────────────────────────────────────────────────────────
const query = new URLSearchParams(location.search);
const fromQuery = query.get("lang");
// `?capture`: screenshots and the walkthrough show the page exactly as a box
// on a wall shows it — no demo controls — and set the time from outside.
const capture = query.has("capture");
let lang = fromQuery || (() => { try { return localStorage.getItem("hems.lang"); } catch { return null; } })()
  || ((navigator.language || "de").startsWith("de") ? "de" : "en");
// The access code lives in this variable from sign-in until the page is closed,
// and nowhere a later script could find it: the sign-in QR needs it once more.
let transfer = null;
let code = null, order = "", sheet = null, signinQr = null, trust = null, found = null;
let supplyStatus = null, supplyDraft = null, supplyMessage = "", supplyBusy = false;
let finding = false, installationOutcome = null;
let push = null, sub = null, hooks = null, hookSecret = null;
let days = null, monthKey = null, picked = null;
let home = null, pairing = null, view = "now", focus = 0, at = null, playing = false, stale = false, lastOk = null;
let missedPolls = 0;
const VIEWS = ["now", "plan", "day", "setup", "glance", "handover", "record"];
const $ = (s) => document.querySelector(s);
const t = (k, p = {}) => String(T[lang][k] ?? T.en[k] ?? k).replace(/\{(\w+)\}/g, (_, n) => p[n] ?? "");
const locale = () => (lang === "de" ? "de-DE" : "en-GB");
const num = (v, d = 1) => v == null ? "–" : Number(v).toLocaleString(locale(), { minimumFractionDigits: d, maximumFractionDigits: d });
const eur = (v) => v == null ? "–" : Number(v).toLocaleString(locale(), { style: "currency", currency: "EUR" });
const hhmm = (iso) => new Date(iso).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit", timeZone: "Europe/Berlin" });
const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const minToHHMM = (m) => `${String((m / 60) | 0).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`;
// Monochrome glyphs, drawn rather than emoji, so they take the device's colour.
const GLYPH = {
  car: '<path d="M5 16h14l-1.6-5.2A2 2 0 0 0 15.5 9.4h-7a2 2 0 0 0-1.9 1.4L5 16Zm0 0v3m14-3v3M7.5 13.5h.01M16.5 13.5h.01" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round"/>',
  heat_pump: '<path d="M8 20c-2-3 2-5 0-8m4 8c-2-3 2-5 0-8m4 8c-2-3 2-5 0-8M6 6h12" stroke="currentColor" stroke-width="2" fill="none" stroke-linecap="round"/>',
  battery: '<rect x="4" y="7" width="15" height="10" rx="2" stroke="currentColor" stroke-width="2" fill="none"/><path d="M21 10v4M7 10v4m3-4v4" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>',
  hot_water: '<path d="M12 3s6 6.5 6 11a6 6 0 0 1-12 0c0-4.5 6-11 6-11Z" stroke="currentColor" stroke-width="2" fill="none"/>',
  appliance: '<rect x="5" y="3" width="14" height="18" rx="2" stroke="currentColor" stroke-width="2" fill="none"/><circle cx="12" cy="13" r="4" stroke="currentColor" stroke-width="2" fill="none"/>',
  roof: '<circle cx="12" cy="12" r="4" stroke="currentColor" stroke-width="2" fill="none"/><path d="M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.4 1.4m11.2 11.2L19 19M5 19l1.4-1.4M17.6 6.4 19 5" stroke="currentColor" stroke-width="2" stroke-linecap="round"/>',
};
const glyph = (kind) => `<svg viewBox="0 0 24 24" aria-hidden="true">${GLYPH[kind] ?? GLYPH.appliance}</svg>`;

function why(e) {
  const params = Object.fromEntries(Object.entries(e.params || {}).map(([k, v]) => [k, k === "kw" ? num(v) : v]));
  const s = (T[lang].why[e.key] ?? T.en.why[e.key] ?? e.key).replace(/\{(\w+)\}/g, (_, n) => params[n] ?? "");
  return esc(s) + (e.citation ? `<span class="cite">${esc(e.citation)}</span>` : "");
}
const pretty = (id) => id.charAt(0).toUpperCase() + id.slice(1).replace(/ue/g, "ü").replace(/ae/g, "ä").replace(/oe/g, "ö").replace(/[_-]/g, " ");
function label(h, d) {
  if (d.name) return d.name;
  const same = h.devices.filter((x) => x.kind === d.kind).length > 1;
  return same ? pretty(d.id) : t(d.kind);
}
function toast(msg) { const el = $("#toast"); el.textContent = msg; el.hidden = false; clearTimeout(toast.h); toast.h = setTimeout(() => (el.hidden = true), 2800); }

// Cheap means cheap *for this household*: the cheapest third of its own plan.
function cheapSlots(h) {
  const prices = h.plan.map((s) => s.price_ct).filter((p) => p != null).sort((a, b) => a - b);
  if (!prices.length) return h.plan.map(() => false);
  const cut = prices[Math.max(0, Math.floor(prices.length / 3) - 1)];
  return h.plan.map((s) => s.price_ct != null && s.price_ct <= cut + 1e-9);
}
// Runs of `true` in the first `span` entries, as [start, end) pairs.
function runs(flags, span) {
  const out = []; let i = 0;
  while (i < span) { if (flags[i]) { let j = i; while (j < span && flags[j]) j++; out.push([i, j]); i = j; } else i++; }
  return out;
}

// ── The box ─────────────────────────────────────────────────────────────────
async function api(path, opts = {}) {
  const r = await fetch(path, { credentials: "same-origin", headers: { "content-type": "application/json" }, ...opts });
  if (r.status === 401) {
    const hadSession = home !== null;
    home = code = null; view = "signin"; render();
    if (hadSession) location.reload();
    throw new Error("unauthorised");
  }
  if (!r.ok) {
    // Every refusal is an RFC 9457 problem document; `detail` is the sentence.
    const problem = await r.json().catch(() => null);
    throw Object.assign(new Error(problem?.detail || r.statusText), { status: r.status });
  }
  return r.status === 204 ? null : r.json();
}
async function signIn(token) {
  const r = await fetch("/v1/session", { method: "POST", credentials: "same-origin", headers: { "content-type": "application/json" }, body: JSON.stringify({ token }) });
  if (r.ok) code = token;
  return r.ok;
}
async function load() {
  try {
    home = await api(home?.demo && at != null ? `/v1/home?at=${at}` : "/v1/home");
    missedPolls = 0; stale = false; lastOk = new Date();
    if (view === "signin") view = location.hash.slice(1) || "now";
    if (view === "setup" && !home.demo) {
      const [diagnostics, peers, change, supply] = await Promise.all(["/v1/trust", "/v1/pairing", "/v1/commissioning/change", "/v1/tariff"].map(path => api(path).catch(() => null)));
      trust = diagnostics; pairing = peers; supplyStatus = supply;
      transfer = home.shadow ? await api("/v1/commissioning/transfer").catch(() => null) : null;
      if (change && installationOutcome?.outcome !== change.outcome) {
        installationOutcome = change;
        if (change.outcome !== "restarting") found = null;
        render(true);
      }
    }
    if (home.demo && at == null) at = home.at_minute ?? 0;
  } catch { if (view !== "signin") { missedPolls++; stale = true; } }
  render();
}

// ── Now ─────────────────────────────────────────────────────────────────────
function sentence(h) {
  if (h.shadow) return t("shadowReads");
  if (!h.at) return t("sNoData");
  const r = h.reduction, g = h.grid_kw ?? 0, ct = h.price_ct != null ? num(h.price_ct, 0) : "–";
  if (r?.failsafe) return t("sFailsafe");
  if (r) return t(h.devices.some((d) => d.kind === "battery" && (d.kw ?? 0) < -0.05) ? "sReducedCovered" : "sReduced");
  if (g < -0.05) return t("sFeed", { kw: num(-g) });
  const charging = h.devices.find((d) => d.why.key === "plan.charge" && (d.kind === "car" || d.kind === "battery"));
  if (charging) return t("sCharging", { dev: t(charging.kind), ct });
  const running = h.devices.find((d) => d.kind !== "roof" && (d.kw ?? 0) > 0.05 && d.why.cause === "plan");
  if (running && cheapSlots(h)[0]) return t("sCheapRun", { dev: t(running.kind), ct });
  if (h.devices.some((d) => d.kind === "battery" && (d.kw ?? 0) < -0.05)) return t("sCovering", { ct });
  return t("sPlain");
}
function cheapBand(h) {
  if (!h.plan.length) return "";
  const cheap = cheapSlots(h), n = h.plan.length, span = Math.min(n, 96);
  let title, sub = "";
  if (cheap[0]) {
    const end = cheap.findIndex((c) => !c);
    title = t("cheapNow"); sub = end > 0 ? t("cheapUntil", { t: hhmm(h.plan[end].start) }) : "";
  } else {
    const start = cheap.indexOf(true);
    if (start < 0) title = t("cheapNone");
    else { const len = cheap.slice(start).findIndex((c) => !c); title = t("cheapNext", { t: hhmm(h.plan[start].start) }); sub = t("cheapFor", { h: num(((len < 0 ? n - start : len) * 15) / 60, 0) }); }
  }
  // Only the cheap windows are drawn, the way Grid Forecast draws only the
  // cleaner times; the operator's reductions are hatched over them.
  const pct = (k) => (k / span) * 100;
  const seg = ([a, b], cls = "") => `<i class="${cls}" style="left:${pct(a)}%;width:${pct(b - a)}%"></i>`;
  return `<div class="band"><div class="line"><b>${title}</b><span>${sub}</span></div>
  <div class="bar">${runs(cheap, span).map((r) => seg(r)).join("")}${runs(h.plan.map((s) => s.reduced), span).map((r) => seg(r, "op")).join("")}</div>
  <div class="ticks">${[0, 24, 48, 72].map((k) => `<span>${h.plan[k] ? hhmm(h.plan[k].start) : ""}</span>`).join("")}</div></div>`;
}
function car(h) {
  const d = h.devices.find((x) => x.kind === "car"); if (!d) return "";
  const p = d.soc ?? 0, charging = (d.kw ?? 0) > 0.05;
  const g = h.goal?.[0], a = h.goal?.[1];
  const goalLine = g ? `${t("carGoal", { pct: Math.round(g.target_soc * 100), t: minToHHMM(g.by_minute) })}${a ? `<span class="verdict v-${a.verdict}">${t("v_" + a.verdict)}</span>` : ""}` : `<a href="#plan" style="color:var(--accent);text-decoration:none">${t("carNoGoal")} ›</a>`;
  return `<h2 class="sec">${t("car")}</h2><div class="car"><div class="ring" style="--p:${p}"><span class="num">${d.soc != null ? Math.round(p * 100) + "%" : "–"}</span></div>
  <div><b>${charging ? t("carCharging", { kw: num(d.kw) }) : t("carIdle")}</b><p>${goalLine}</p></div></div>`;
}
function row(h, d) {
  if (h.shadow) return `<div class="row"><span class="glyph">${glyph(d.kind)}</span><span class="name">${esc(label(h, d))}</span><span class="val num">${d.measured_w == null ? "–" : num(d.measured_w / 1000) + " kW"}</span></div>`;
  return `<div class="row"><span class="glyph ${d.why.key === "idle" ? "" : d.why.cause}">${glyph(d.kind)}</span>
  <span class="name">${esc(label(h, d))}</span><span class="val num">${Math.abs(d.kw ?? 0) > 0.05 ? num(d.kw) + " kW" : ""}</span>
  <span class="why">${why(d.why)}</span>${d.notice ? `<span class="notice">${t("notice_" + d.notice)}</span>` : ""}
  ${d.kind === "hot_water" && h.tank_goal ? `<span class="why">${t("tankGoal", { c: num(h.tank_goal[0].target_c, 0), t: minToHHMM(h.tank_goal[0].by_minute) })}<span class="verdict v-${h.tank_goal[1]?.verdict}">${h.tank_goal[1] ? t("v_" + h.tank_goal[1].verdict) : ""}</span></span>` : ""}</div>`;
}
// While § 14a is in force: who gets what of the ceiling, by need, from the
// guard's own verdict. A device that only measures takes no share.
function shares(h) {
  const r = h.reduction, entries = Object.entries(r?.shares ?? {});
  if (!entries.length) return "";
  const of = (id) => h.devices.find((d) => d.id === id) ?? { id, kind: "appliance" };
  const budget = r.budget_kw ?? r.ceiling_kw, scale = Math.max(budget, 0.001);
  return `<h2 class="sec">${t("shared", { kw: num(budget) })}</h2><div class="group shares">${entries.map(([id, kw]) =>
    `<div class="row"><span class="glyph">${glyph(of(id).kind)}</span><span class="why"><b>${esc(label(h, of(id)))}</b><i class="share" style="--w:${Math.min(100, (Math.max(kw, 0) / scale) * 100).toFixed(1)}%"></i></span><span class="num">${num(kw)} kW</span></div>`).join("")}</div>
  <p class="cap" style="margin:8px 16px 0">${t("sharedHelp")}</p>`;
}
function reductionChip(r) {
  if (!r) return "";
  const label = r.failsafe ? "chipFailsafe" : r.minimum_kw == null ? "chipCeiling" : "chipReduced";
  const kw = label === "chipReduced" ? r.minimum_kw : r.ceiling_kw;
  return `<span class="chip">${t(label, { kw: num(kw) })}</span>`;
}
function viewNow(h) {
  const g = h.grid_kw, r = h.reduction;
  // Resting means drawing nothing as well as being told nothing.
  const still = (d) => !h.shadow && d.why.key === "idle" && !d.notice && Math.abs(d.kw ?? 0) <= 0.05;
  const active = h.devices.filter((d) => d.kind !== "car" && !still(d));
  const resting = h.devices.filter((d) => d.kind !== "car" && still(d));
  return `<section class="hero"><div class="figure num">${g == null ? "–" : num(Math.abs(g))}<small>kW</small></div>
  <div class="cap">${g == null ? t("sNoData") : t(g < -0.05 ? "toGrid" : "fromGrid")}${h.price_ct != null ? ` · <b>${num(h.price_ct, 0)} ct</b> ${t("perKwh")}` : ""}</div>
  <p class="say">${sentence(h)}</p>
  ${reductionChip(r)}</section>
  ${shares(h)}
  ${h.plan.length ? `<h2 class="sec">${t("next24")}</h2>${cheapBand(h)}` : ""}
  ${car(h)}
  <h2 class="sec">${t("devices")}</h2><div class="group">${active.map((d) => row(h, d)).join("")}
  ${resting.length ? `<div class="row quiet"><span class="glyph">${glyph("appliance")}</span><span class="why"><b style="color:var(--ink);font-weight:600">${t("resting")}</b> · ${resting.map((d) => esc(label(h, d))).join(", ")}</span></div>` : ""}</div>
  ${h.roof ? `<p class="cap" style="margin:12px 16px">${t("roof_" + h.roof)}</p>` : ""}`;
}

// ── Glance ──────────────────────────────────────────────────────────────────
// Now, for a wall: the figure readable across a room, no controls, the screen
// kept awake. A tap anywhere leads back, since a home-screen app has no address bar.
function viewGlance(h) {
  const g = h.grid_kw, r = h.reduction, d = h.devices.find((x) => x.kind === "car");
  // A still image burns into a display that shows it for a year; drift a few pixels a minute.
  const m = h.at ? new Date(h.at).getUTCMinutes() : 0;
  return `<a class="face" href="#now" title="${t("tab.now")}" style="transform:translate(${(m % 7) - 3}px,${(m % 5) - 2}px)">
  <div class="gtop"><span class="num">${h.at ? hhmm(h.at) : ""}</span>${reductionChip(r)}</div>
  <div class="mid"><div class="figure num">${g == null ? "–" : num(Math.abs(g))}<small>kW</small></div>
  <div class="cap">${g == null ? t("sNoData") : t(g < -0.05 ? "toGrid" : "fromGrid")}${h.price_ct != null ? ` · <b>${num(h.price_ct, 0)} ct</b> ${t("perKwh")}` : ""}</div>
  <p class="say">${missedPolls >= 3 ? t("stale", { t: lastOk ? lastOk.toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" }) : "–" }) : sentence(h)}</p></div>
  <div class="gfoot">${cheapBand(h)}${d ? `<div class="ring" style="--p:${d.soc ?? 0}"><span class="num">${d.soc != null ? Math.round(d.soc * 100) + "%" : "–"}</span></div>` : ""}</div></a>`;
}
let lock = null, requestingLock = false;
async function awake(on) {
  const visible = () => view === "glance" && document.visibilityState === "visible";
  try {
    if (!on || !visible()) { await lock?.release(); return; }
    if (lock || requestingLock || !navigator.wakeLock) return;
    requestingLock = true;
    try {
      const next = await navigator.wakeLock.request("screen");
      if (!visible()) { await next.release(); return; }
      lock = next;
      next.addEventListener("release", () => { if (lock === next) lock = null; });
    } finally { requestingLock = false; }
  } catch { /* The display can dim when wake lock is unavailable. */ }
}
document.addEventListener("visibilitychange", () => awake(view === "glance"));

// ── Plan ────────────────────────────────────────────────────────────────────
function strip(h) {
  const slots = h.plan, n = slots.length, cheap = cheapSlots(h);
  const W = 358, H = 168, top = 8, hh = H - top - 22, bw = W / n;
  const totals = slots.map((s) => Object.values(s.kw).reduce((a, b) => a + Math.max(0, b), 0));
  const max = Math.max(4, ...totals);
  let svg = `<svg class="strip" id="strip" viewBox="0 0 ${W} ${H}" role="img" aria-label="${t("planTitle")}">
  <defs><pattern id="hatch" width="5" height="5" patternUnits="userSpaceOnUse" patternTransform="rotate(45)"><line x1="0" y1="0" x2="0" y2="5" stroke="var(--op)" stroke-width="1.8"/></pattern></defs>
  <rect x="0" y="${top}" width="${W}" height="${hh}" rx="10" fill="var(--card)"/>`;
  slots.forEach((s, i) => {
    if (cheap[i]) svg += `<rect x="${i * bw}" y="${top}" width="${bw + .4}" height="${hh}" fill="var(--cheap-soft)"/>`;
    if (s.reduced) svg += `<rect x="${i * bw}" y="${top}" width="${bw + .4}" height="${hh}" fill="url(#hatch)" opacity=".55"/>`;
    const bh = (totals[i] / max) * (hh - 10);
    if (bh > 0.2) svg += `<rect x="${i * bw + .5}" y="${top + hh - bh}" width="${Math.max(.8, bw - 1)}" height="${bh}" rx="1.2" fill="${i === focus ? "var(--accent)" : "var(--ink3)"}"/>`;
  });
  for (let i = 0; i < n; i += 24) svg += `<text x="${Math.min(i * bw + 2, W - 32)}" y="${H - 6}" font-size="11" fill="var(--ink2)" font-family="-apple-system,system-ui,sans-serif">${hhmm(slots[i].start)}</text>`;
  svg += `<line x1="${focus * bw + bw / 2}" x2="${focus * bw + bw / 2}" y1="${top}" y2="${top + hh}" stroke="var(--accent)" stroke-width="1.5"/></svg>`;
  return svg;
}
// A goal is asked of a device by its kind: a charge for the car, a temperature
// for the tank. The answer comes back at once, from the planner's own model.
function capLabel(value) {
  return value == null ? "" : " · ≤ " + num(value, 0) + " ct/kWh";
}
function goalCard(h, kind) {
  const d = h.devices.find((x) => x.kind === kind); if (!d) return "";
  const tank = kind === "hot_water", pair = tank ? h.tank_goal : h.goal, a = pair?.[1];
  const g = pair?.[0] ?? (tank ? { target_c: 50, by_minute: 1080 } : { target_soc: 0.8, by_minute: 420 });
  const v = Math.round(tank ? g.target_c : g.target_soc * 100), unit = tank ? "°C" : "%";
  const times = [360, 390, 420, 450, 480, 540, 1020, 1080, 1200].concat(g.by_minute).filter((m, i, l) => l.indexOf(m) === i).sort((x, y) => x - y);
  const reach = tank ? `${num(a?.reachable_c ?? 0, 0)} °C` : `${Math.floor((a?.reachable_soc ?? 0) * 100)} %`;
  return `<h2 class="sec">${t(tank ? "goalTank" : "goal")}</h2><div class="group goal" data-goal="${esc(d.id)}" data-tank="${tank ? 1 : ""}" data-unit="${unit}">
  <div class="row cap">${pair ? t("qSaved") + " · " + v + " " + unit + " · " + minToHHMM(g.by_minute) + capLabel(g.price_cap_ct) : t("qNone")}</div>
  <div class="row"><label>${t(tank ? "goalHeat" : "goalTo")}</label><span class="goal-value"><input type="number" inputmode="numeric" data-amount aria-label="${t(tank ? "goalHeat" : "goalTo")} (${unit})" min="${tank ? 40 : 20}" max="${tank ? 70 : 100}" step="1" value="${v}" required> ${unit}</span></div>
  <div class="row" style="padding-top:0"><input type="range" min="${tank ? 40 : 20}" max="${tank ? 70 : 100}" step="1" value="${v}" aria-label="${t(tank ? "goalHeat" : "goalTo")}" style="grid-column:1/3"></div>
  <div class="row"><label>${t("goalBy")}</label><select aria-label="${t("goalBy")}">${times.map((m) => `<option value="${m}" ${m === g.by_minute ? "selected" : ""}>${minToHHMM(m)}</option>`).join("")}</select></div>
  ${tank ? "" : `<div class="row"><label for="goalCap">${t("qCap")}</label><span class="goal-value"><input type="number" id="goalCap" data-cap min="-2147483648" max="2147483647" step="1" value="${g.price_cap_ct ?? ""}"> ct/kWh</span></div>`}
  ${a ? `<div class="row" data-answer><span><span class="verdict v-${a.verdict}" style="margin:0 8px 0 0">${t("v_" + a.verdict)}</span>${t("vt_" + a.verdict, { kwh: num(a.needed_kwh), reach })}${a.eligible_quarters == null ? "" : " · " + t("qEligible", {n:a.eligible_quarters, cap:g.price_cap_ct})}</span></div>` : ""}
  <div class="row quote" data-quote hidden></div><button type="button" class="link" data-cancel hidden>${t("qCancel")}</button>
  ${!tank && pair && !h.demo && !h.shadow && (h.charging_now || (a && a.verdict !== "met")) ? `<button class="link" data-boost type="button">${t(h.charging_now ? "qStopBoost" : "qBoostReview")}</button>` : ""}
  ${pair ? `<button class="link danger" type="button" data-clear>${t("goalClear")}</button>` : ""}</div>`;
}
// The plan in at most three sentences: each controllable device's first run in
// the next 24 hours, from the plan's own slots. A device with nothing planned
// has no sentence.
function tonight(h) {
  const slots = h.plan.slice(0, 96), out = [];
  const run = (id, test) => {
    const i = slots.findIndex((s) => test(s.kw[id] ?? 0)); if (i < 0) return null;
    let j = i; while (j + 1 < slots.length && test(slots[j + 1].kw[id] ?? 0)) j++;
    const end = new Date(new Date(slots[j].start).getTime() + 15 * 60000).toISOString();
    return { a: hhmm(slots[i].start), b: hhmm(end), at: i };
  };
  for (const d of h.devices) {
    const say = (key, r) => r && out.push({ text: t(key, r), at: r.at });
    if (d.kind === "car") say("runCar", run(d.id, (kw) => kw > 0.05));
    else if (d.kind === "hot_water") say("runHot", run(d.id, (kw) => kw > 0.05));
    else if (d.kind === "heat_pump") say("runHeat", run(d.id, (kw) => kw > 0.05));
    else if (d.kind === "battery") {
      const i = run(d.id, (kw) => kw > 0.05), o = run(d.id, (kw) => kw < -0.05);
      const first = [i && { key: "runBatteryIn", r: i }, o && { key: "runBatteryOut", r: o }].filter(Boolean).sort((x, y) => x.r.at - y.r.at)[0];
      if (first) say(first.key, first.r);
    }
  }
  return out.sort((x, y) => x.at - y.at).slice(0, 3).map((x) => x.text).join(" ");
}
// The export-or-store decision with both prices: the number closed systems withhold.
function worth(h) {
  const w = h.worth; if (!w) return "";
  return t(w.battery_ct >= w.export_ct ? "worthStore" : "worthSell", { b: num(w.battery_ct, 0), e: num(w.export_ct, 0) });
}
function forecastLearning() {
  if (!days) return "";
  const count = days.filter(d => d.forecast != null).length;
  return count < 14 ? `<p class="cap">${t("learningDays", { n: count })}</p>` : "";
}
function viewPlan(h) {
  if (!h.plan.length) return `<div class="group"><div class="slot"><p>${why({ key: "fallback.no_plan" })}</p></div></div>${forecastLearning()}`;
  const i = Math.min(focus, h.plan.length - 1), s = h.plan[i];
  const running = Object.entries(s.kw).filter(([, kw]) => Math.abs(kw) > 0.05);
  const energy = running.reduce((a, [, kw]) => a + Math.max(0, kw) * 0.25, 0);
  const best = Math.min(...h.plan.map((p) => p.price_ct ?? Infinity));
  const diff = s.price_ct != null && isFinite(best) ? ((s.price_ct - best) / 100) * energy : null;
  const name = (id) => esc(label(h, h.devices.find((d) => d.id === id) ?? { kind: "appliance", id }));
  const said = [tonight(h), worth(h)].filter(Boolean).join(" ");
  // The legend names only colours the strip uses: an operator's reduction is
  // never announced ahead, so most strips carry none.
  const op = h.plan.some((p) => p.reduced);
  return `<h2 class="sec">${t(h.shadow ? "shadowPlan" : "planTitle")}</h2>${strip(h)}
  <div class="legend"><span><i style="background:var(--cheap)"></i>${t("legendCheap")}</span><span><i style="background:var(--ink3)"></i>${t("legendDraw")}</span>${op ? `<span><i style="background:var(--op)"></i>${t("legendOp")}</span>` : ""}</div>
  ${h.previous_plan_at ? `<p class="cap" role="status">${t("previousPlan", {time: esc(new Date(h.previous_plan_at).toLocaleString(locale(), {dateStyle:"short", timeStyle:"short", timeZone:"Europe/Berlin"}))})}</p>` : ""}
  ${said ? `<p class="say plan">${said}</p>` : ""}
  ${h.ahead ? `<p class="cap" style="margin:10px 4px 0">${t(h.ahead.bill_eur < 0 ? "aheadEarn" : "aheadBill", { t: new Date(h.ahead.until).toLocaleString(locale(), { weekday: "short", hour: "2-digit", minute: "2-digit", timeZone: "Europe/Berlin" }), eur: eur(Math.abs(h.ahead.bill_eur)) })}</p>` : ""}
  <div class="group" style="margin-top:14px"><div class="slot"><span class="when num">${hhmm(s.start)} · ${s.price_ct != null ? num(s.price_ct, 0) + " ct/kWh" : "–"}${s.reduced ? ` · <span style="color:var(--op)">${t("reducedSlot")}</span>` : ""}</span>
  <h3>${running.length ? running.map(([id, kw]) => `${name(id)} <span class="num" style="color:var(--ink2);font-weight:400">${num(kw)} kW</span>`).join(" · ") : t("nothing")}</h3>
  ${running.length ? "" : `<p>${t("nothingWhy")}</p>`}
  ${energy > 0 && diff != null ? `<p class="contrast">${diff > 0.005 ? t("contrast", { best: num(best, 0), diff: eur(diff) }) : t("contrastBest")}</p>` : ""}</div></div>
  ${goalCard(h, "car")}${goalCard(h, "hot_water")}${forecastLearning()}`;
}

// ── Day ─────────────────────────────────────────────────────────────────────
// A closed day from /v1/days, in the receipt's own terms. Money is whole cents
// on the box, so a month adds up to the cent.
function receiptOf(d) {
  const m = d.money, e = (k) => (m ? m[k] / 100 : null);
  return { day: d.day, priced: !!m, imported_kwh: d.imported_wh / 1000, produced_kwh: d.produced_wh / 1000, bill_eur: e("bill_ct"), wear_eur: e("wear_ct"), comfort_eur: e("comfort_ct"),
    cost_eur: e("cost_ct"), without_eur: e("without_ct"), saved_eur: e("saved_ct"), on_bill_eur: e("on_bill_ct"),
    reduced_minutes: d.reduced_minutes, reductions: d.reductions, reduction_respected: d.reduction_respected, feed_in_respected: d.feed_in_respected };
}
// One bar per calendar day: the saving, down and red where the day lost. A day
// the box was off is a gap, never a zero.
function month() {
  if (!days?.length) return "";
  const months = [...new Set(days.map((d) => d.day.slice(0, 7)))];
  const key = months.includes(monthKey) ? monthKey : months.at(-1), at = months.indexOf(key);
  const [y, mo] = key.split("-").map(Number), len = new Date(Date.UTC(y, mo, 0)).getUTCDate();
  const by = new Map(days.filter((d) => d.day.startsWith(key)).map((d) => [+d.day.slice(8), d]));
  const priced = [...by.values()].filter((d) => d.money), sum = (k) => priced.reduce((a, d) => a + d.money[k], 0);
  const bills = [...by.values()].map(d => d.metered_bill).filter(b => b?.energy_ct != null && b.fixed_ct != null);
  const billed = bills.reduce((cents, b) => cents + b.energy_ct + b.fixed_ct, 0);
  const delta = d => d.comparison ? d.comparison.without_plan_ct - d.comparison.with_plan_ct : d.money?.saved_ct;
  const blocks = [...by.values()].map(d => d.arbitrage).filter(Boolean);
  const matched = [...by.values()].filter(d => d.comparison && Math.abs(delta(d)) > d.comparison.error_ct);
  const values = [...by.values()].map(delta).filter(v => v != null);
  const W = 358, H = 132, top = 6, area = H - top - 20, neg = values.some(v => v < 0);
  const up = neg ? area * 0.68 : area, y0 = top + up, max = Math.max(50, ...values.map(Math.abs)), bw = W / len;
  let svg = `<svg class="strip" viewBox="0 0 ${W} ${H}" role="group" aria-label="${t("month")}"><line x1="0" x2="${W}" y1="${y0}" y2="${y0}" stroke="var(--sep)"/>`;
  for (let i = 1; i <= len; i++) {
    const d = by.get(i), x = (i - 1) * bw;
    if (i % 7 === 1) svg += `<text x="${x + bw / 2}" y="${H - 4}" text-anchor="middle" font-size="11" fill="var(--ink2)" font-family="-apple-system,system-ui,sans-serif">${i}</text>`;
    if (!d) continue;
    const sel = d.day === picked;
    if (delta(d) == null) svg += `<circle cx="${x + bw / 2}" cy="${y0}" r="2" fill="none" stroke="var(--ink3)"/>`;
    else { const v = delta(d), h = Math.max(1.5, (Math.abs(v) / max) * (v < 0 ? area - up : up - 4)); svg += `<rect x="${x + 1}" y="${v < 0 ? y0 : y0 - h}" width="${Math.max(1, bw - 2)}" height="${h}" rx="1.5" fill="${d.comparison && Math.abs(v) <= d.comparison.error_ct ? "var(--ink3)" : v < 0 ? "var(--bad)" : "var(--accent)"}" opacity="${picked && !sel ? .35 : 1}"/>`; }
    svg += `<rect role="button" tabindex="0" aria-label="${d.day}" aria-pressed="${sel}" data-day="${d.day}" x="${x}" y="0" width="${bw}" height="${H}" fill="transparent"/>`;
  }
  const title = new Date(Date.UTC(y, mo - 1, 15)).toLocaleDateString(locale(), { month: "long", year: "numeric" });
  const nav = (k, label, ch) => `<button type="button" class="step" ${k ? `data-month="${k}"` : "disabled"} aria-label="${label}">${ch}</button>`;
  return `<h2 class="sec">${t("month")}</h2><div class="month"><div class="mhead">${nav(months[at - 1], t("earlier"), "‹")}<b>${title}</b>${nav(months[at + 1], t("later"), "›")}</div>
  ${priced.length ? `<div class="mtotal num">${eur(sum("saved_ct") / 100)}</div><div class="cap">${t("savedOver", { n: priced.length })} · ${t("monthCost", { c: eur(sum("cost_ct") / 100), w: eur(sum("without_ct") / 100) })}</div>` : ""}
  ${bills.length ? `<div class="mtotal num">${eur(billed / 100)}</div><div class="cap">${t("monthlyBill", {n: bills.length})}</div>` : ""}
  ${blocks.length ? `<div class="mtotal num">${eur(blocks.reduce((sum, b) => sum + b.avoided_ct - b.bought_ct, 0) / 100)}</div><div class="cap">${t("blockMargin")} · ${t("blockCount", {n: blocks.reduce((n, b) => n + b.blocks, 0)})}</div>` : ""}
  ${matched.length ? `<div class="mtotal num">${eur(matched.reduce((sum, d) => sum + delta(d), 0) / 100)} ± ${eur(matched.reduce((sum, d) => sum + d.comparison.error_ct, 0) / 100)}</div><div class="cap">${t("matchedDays", {n: matched.length})}</div>` : ""}
  ${svg}</svg></div>`;
}
function viewDay(h) {
  const chosen = picked && days?.find((d) => d.day === picked);
  const day = chosen || h.receipt;
  const r = day ? receiptOf(day) : null;
  const li = (a, b, c = "") => `<div class="li ${c}"><span>${a}</span><span>${b}</span></div>`;
  const yes = (v) => `<span class="${v ? "ok" : "no"}">${t(v ? "yes" : "no")}</span>`;
  // What the day left in or took out of the stores is part of its cost; without
  // the line the receipt would not add up.
  const rest = r && r.priced !== false && r.cost_eur != null ? r.cost_eur - r.bill_eur - r.wear_eur - r.comfort_eur : 0;
  const bill = day?.metered_bill;
  const metered = !bill ? "" : bill.energy_ct == null || bill.fixed_ct == null
    ? `<p class="cap">${t("pricedCoverage", {n: bill.priced_intervals, total: bill.expected_intervals})}</p>`
    : li(t("measuredCost"), eur(bill.energy_ct / 100)) + li(t("fixedCharges"), eur(bill.fixed_ct / 100)) + li(t("bill"), eur((bill.energy_ct + bill.fixed_ct) / 100), "big");
  const c = day?.comparison;
  const diff = c && c.without_plan_ct - c.with_plan_ct;
  const model = c ? li(t(bill?.watched_intervals ? "planModel" : "idleModel"), eur((bill?.watched_intervals ? c.with_plan_ct : c.without_plan_ct) / 100))
    + li(t("replayErr"), eur(c.error_ct / 100))
    + li(t("netDifference"), Math.abs(diff) <= c.error_ct ? t("similarBill") : eur(diff / 100), "big")
    : bill?.fixed_ct != null ? `<p class="cap">${t("noComparison")}</p>` : "";
  const a = day?.arbitrage;
  const blockLine = (label, cents, wh, rate) => li(label, eur(cents / 100))
    + `<p class="cap">${num(wh / 1000)} kWh × ${num(rate / 1000, 2)} ct/kWh</p>`;
  const storage = !a ? "" : blockLine(t("boughtBlock"), a.bought_ct, a.bought_wh, a.buy_rate_mct_per_kwh)
    + blockLine(t("servedBlock"), a.avoided_ct, a.served_wh, a.use_rate_mct_per_kwh)
    + li(t("blockMargin"), eur((a.avoided_ct - a.bought_ct) / 100), "big");
  const watched = bill?.watched_intervals ? `<p class="cap">${t("watchedIntervals", {n: bill.watched_intervals})}</p>` : "";
  const bon = !r ? `<div class="group"><div class="slot"><p>${t("noReceipt")}</p></div></div>` : `<div class="receipt"><h2>${t("receipt")}</h2><div class="sub">${new Date(r.day + "T12:00:00Z").toLocaleDateString(locale(), { weekday: "long", day: "numeric", month: "long" })}${h.demo ? " · demo" : ""}</div>
  ${(bill?.contracts ?? []).map(contract => li(t("supply"), esc(contract))).join("")}
  ${li(t("bought"), num(r.imported_kwh) + " kWh")}${li(t("made"), num(r.produced_kwh) + " kWh")}<div class="rule"></div>
  ${r.priced === false ? metered + model + storage : `${li(t("bill"), eur(r.bill_eur), "big")}${li(t("wear"), eur(r.wear_eur))}${li(t("comfort"), eur(r.comfort_eur))}${Math.abs(rest) >= 0.005 ? li(t("stores"), eur(rest)) : ""}
  ${li(t("cost"), eur(r.cost_eur), "big")}${li(t("without"), eur(r.without_eur))}<div class="rule"></div>
  ${li(t("saved"), eur(r.saved_eur), "save")}${li(t("onBill"), eur(r.on_bill_eur))}<div class="rule"></div>`}
  ${li(t("reductionMin"), r.reduced_minutes != null ? r.reduced_minutes + " min" : "× " + r.reductions)}${li(t("respected"), yes(r.reduction_respected))}${li(t("feedRespected"), yes(r.feed_in_respected))}</div>
  ${chosen ? `<p style="margin:10px 0 0"><button class="link" id="latest" type="button">${t("latestDay")}</button></p>` : ""}`;
  const coverage = day?.forecast;
  const comparison = coverage ? `<p class="cap">${t("forecastCoverage")} · ${t("forecastBands", {pv: num(coverage.pv_pct, 0), load: num(coverage.load_pct, 0)})}</p>` : "";
  return `${bon}${watched}${comparison}${forecastLearning()}${h.levies_year ? `<p class="cap" style="margin:10px 16px 0;color:var(--warn)">${t("leviesYear", { y: h.levies_year })}</p>` : ""}${month()}
  ${h.demo ? "" : `<details id="gridHistory"><summary>${t("gridHistory")}</summary><div class="history-body"></div></details>`}
  <details><summary>${t("method")}</summary><p>${t(h.demo ? "methodText" : "measuredMethod")}</p></details>
  ${days?.length || !h.demo ? `<div class="group export-links">${days?.length ? `<a class="link" href="/v1/days.csv" download>${t("csv")}</a>` : ""}${h.demo ? "" : `<button class="link" id="export" type="button">${t("export")}</button><p id="exportStatus" role="status"></p>`}</div>` : ""}`;
}

// ── Setup ───────────────────────────────────────────────────────────────────
// ── Notifications ───────────────────────────────────────────────────────────
// Four occasions, each a switch. Safari on a home-screen page has
// `window.pushManager`; every other browser reaches it through the worker.
const EVENTS = ["reduction", "goal", "device", "failsafe"], NIGHT = { from_minute: 1320, to_minute: 420 };
const pushMgr = async () => window.pushManager ?? ("serviceWorker" in navigator ? (await navigator.serviceWorker.register("/sw.js")).pushManager : null);
const mine = () => push?.subscriptions.find((x) => x.endpoint === sub?.endpoint);
async function loadPush() {
  try { push = await api("/v1/push"); const pm = await pushMgr().catch(() => null); push.can = !!pm; sub = pm ? await pm.getSubscription() : null; } catch { push = null; }
  render();
}
async function savePush(change = {}) {
  const m = mine() ?? { events: EVENTS, quiet: NIGHT };
  await api("/v1/push", { method: "POST", body: JSON.stringify({ ...sub.toJSON(), lang, events: m.events, quiet: m.quiet, ...change }) });
  await loadPush();
}
async function pushOn() {
  try {
    if ((await Notification.requestPermission()) !== "granted") return toast(t("pushDenied"));
    const key = Uint8Array.from(atob(push.key.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
    sub = await (await pushMgr()).subscribe({ userVisibleOnly: true, applicationServerKey: key });
    await savePush();
  } catch (e) { toast(e.message); }
}
async function pushOff() {
  try { await api("/v1/push", { method: "DELETE", body: JSON.stringify({ endpoint: sub.endpoint }) }); } catch { /* already forgotten */ }
  await sub.unsubscribe(); await loadPush();
}
function notifications() {
  if (!push) return "";
  const m = mine(), sw = (id, on, title, help) => `<label class="row sw"><span><b>${title}</b><small>${help}</small></span><input type="checkbox" role="switch" data-push="${id}" ${on ? "checked" : ""}></label>`;
  const body = !push.can ? `<div class="slot"><p>${t("notifyHome")}</p></div>`
    : !m ? `<button class="link" id="pushOn" type="button">${t("notifyOn")}</button>`
    : EVENTS.map((e) => sw(e, m.events.includes(e), t("ev_" + e), t("evd_" + e))).join("") + sw("quiet", !!m.quiet, t("quiet"), t("quietHelp"))
      + `<button class="link" id="pushTest" type="button">${t("pushTest")}</button><button class="link danger" id="pushOff" type="button">${t("pushOff")}</button>`;
  return `<h2 class="sec">${t("notify")}</h2><div class="group">${body}</div><p class="cap" style="margin:8px 16px 0">${t("notifyHelp")}</p>`;
}

// ── Other systems ───────────────────────────────────────────────────────────
// Events out: CloudEvents, signed per Standard Webhooks, to URLs the household
// adds. The secret is in the answer to the add and nowhere else, so it is shown
// once, beside a copy button.
async function loadHooks() { try { hooks = await api("/v1/subscriptions"); } catch { hooks = null; } render(); }
function otherSystems() {
  if (!hooks) return "";
  const what = (h) => (h.types.includes("de.ems.*") ? t("hookAll") : t("hookGrid"));
  const rows = hooks.map((h) => `<div class="slot"><b>${esc(h.name)}</b><small style="display:block;color:var(--ink2);word-break:break-all">${esc(h.url)} · ${what(h)}${h.owed ? ` · ${t("hookOwed", { n: h.owed })}` : ""}</small>
    <div class="peerbtns"><button data-hook-test="${esc(h.id)}" type="button">${t("hookTest")}</button><button data-hook-remove="${esc(h.id)}" type="button">${t("hookRemove")}</button></div></div>`).join("");
  const secret = hookSecret ? `<div class="slot"><p>${t("hookSecret")}</p><code class="secret">${esc(hookSecret)}</code><button class="link" id="hookCopy" type="button">${t("hookCopy")}</button></div>` : "";
  const form = hooks.length < 8 ? `<form class="slot" id="hookAdd"><label class="sec" for="hookName">${t("hookName")}</label><input id="hookName" required maxlength="64" placeholder="Home Assistant">
    <label class="sec" for="hookUrl">${t("hookUrl")}</label><input id="hookUrl" type="url" required inputmode="url" placeholder="homeassistant.local:8123/api/webhook/…">
    <label class="sec" for="hookWhat">${t("hookWhat")}</label><select id="hookWhat"><option value="de.ems.grid.*">${t("hookGrid")}</option><option value="de.ems.*">${t("hookAll")}</option></select>
    <label class="row sw"><span><small>${t("hookPublic")}</small></span><input type="checkbox" role="switch" id="hookPublic"></label>
    <button class="primary" type="submit">${t("hookAdd")}</button></form>` : "";
  return `<h2 class="sec">${t("hooks")}</h2><div class="group">${secret}${rows || (hookSecret ? "" : `<div class="slot"><p>${t("hookNone")}</p></div>`)}${form}</div><p class="cap" style="margin:8px 16px 0">${t("hooksHelp")}</p>`;
}

// ── Handover ────────────────────────────────────────────────────────────────
// One A4 sheet: what the operator's portal asks for, and the household's key.
const groups4 = (s) => (s || "").replace(/[^0-9a-fA-F]/g, "").match(/.{1,4}/g)?.map((g) => `<span>${g}</span>`).join(" ") ?? "–";
async function loadSheet() {
  try { sheet = await api("/v1/handover"); } catch { sheet = null; }
  render(); loadSigninQr();
}
async function loadSigninQr() {
  if (!code || signinQr) return;
  const r = await fetch(`/v1/qr/signin?origin=${encodeURIComponent(location.origin)}`, { headers: { authorization: `Bearer ${code}` } }).catch(() => null);
  if (r?.ok) { signinQr = URL.createObjectURL(await r.blob()); render(); }
}
function viewHandover(h) {
  const s = sheet; if (!s) return "";
  const kw = (w) => num(w / 1000) + " kW", date = (d) => (d ? new Date(d + "T12:00:00Z").toLocaleDateString(locale()) : "–");
  return `<article class="sheet"><header><h2>${t("sheet")}${h.site ? " · " + esc(h.site) : ""}</h2><p class="cap">${t("sheetFor")} · ${new Date(s.generated_at).toLocaleDateString(locale())}</p></header>
  <section class="pair"><img src="/v1/qr/ski" alt="" width="132" height="132"><div><span class="when">${t("ski")}</span><div class="ski">${groups4(s.ski)}</div>
    ${s.ship_id ? `<span class="when">${t("shipId")}</span><div class="mono">${esc(s.ship_id)}</div>` : ""}</div></section>
  <section class="pair"><img src="/v1/qr/fingerprint" alt="" width="132" height="132"><div><span class="when">${t("fp")}</span><div class="ski small">${groups4(s.fingerprint)}</div><p class="cap">${t("fpNote")}</p></div></section>
  ${s.devices.length ? `<h3>${t("sca")}</h3><table><thead><tr><th>${t("thDevice")}</th><th>${t("thPower")}</th><th>${t("thMin")}</th><th>${t("thSince")}</th></tr></thead><tbody>
  ${s.devices.map((d) => `<tr><td><b>${esc(d.labels.map(pretty).join(", "))}</b><small>${t("fg_" + d.fallgruppe)} (${d.letter})</small></td><td class="num">${kw(d.power_w)}</td><td class="num">${kw(d.minimum_direct_w)}</td><td class="num">${date(d.commissioned)}</td></tr>`).join("")}</tbody></table>
  <p class="cap">${t("sheetSum", { min: num(s.minimum_w / 1000), mod: s.module ? t("sheetModule", { m: esc(String(s.module).replace(/\D/g, "") || s.module) }) : "" })}</p>` : ""}
  ${signinQr ? `<section class="pair key"><img src="${signinQr}" alt="" width="132" height="132"><div><b>${t("sheetSignin")}</b><p class="cap">${t("sheetSigninHelp")}</p><p class="only">${t("householdOnly")}</p></div></section>`
    : `<form class="noprint again" id="again"><p class="cap">${t("needCode")}</p><input id="tok2" autocomplete="off" spellcheck="false" aria-label="${t("tokenLabel")}" required><button class="primary" type="submit">${t("ok")}</button></form>`}
  <button class="primary noprint" id="print" type="button">${t("print")}</button></article>`;
}
// ── The completion code ─────────────────────────────────────────────────────
// For whoever ordered the installation. It has a view of its own so it is
// never on screen with the household's sign-in code: one is made to be
// scanned, the other must never be photographed.
function viewRecord() {
  const q = order ? "?reference=" + encodeURIComponent(order) : "";
  return `<article class="sheet"><header><h2>${t("record")}</h2><p class="cap">${t("recordHelp")}</p></header>
  <input class="order" id="order" value="${esc(order)}" maxlength="64" placeholder="${t("orderNo")}" aria-label="${t("orderNo")}" autocomplete="off" autocapitalize="off" spellcheck="false">
  ${order ? "" : `<p class="cap" style="margin:8px 4px 0">${t("recordNone")}</p>`}
  <a class="primary" style="text-align:center;text-decoration:none" href="/v1/commissioning${q}" download="commissioning${order ? "-" + esc(order) : ""}.jws">${t("recordFile")}</a>
  <section class="pair dense"><img id="recordQr" src="/v1/qr/commissioning${q}" alt="${t("record")}" width="300" height="300"><p class="cap" id="recordBig" hidden>${t("recordBig")}</p></section></article>`;
}
async function loadTrust() { try { trust = await api("/v1/trust"); } catch { trust = null; } render(); }
function trustLines() {
  if (!trust) return "";
  const cb = trust.control_box.state, bad = cb === "silent" || cb === "failsafe" || cb === "unpaired";
  return `${trust.identity !== "none" ? `<div class="slot"><span class="when ${trust.identity === "kept" ? "ok" : "no"}">${trust.identity === "kept" ? "✓" : "!"} ${t("id_" + trust.identity)}</span></div>` : ""}
  ${cb !== "none" ? `<div class="slot"><span class="when ${bad ? "no" : "ok"}">${bad ? "!" : "✓"} ${t("trust_" + cb)}</span>${trust.heartbeat_age_s != null ? `<span class="cap"> · ${t("heartbeatAge", {s: num(trust.heartbeat_age_s, 0)})}</span>` : ""}</div>` : ""}`;
}
function discovery() {
  const line = (title, sub, state) => `<div class="slot"><b>${esc(title)}</b><span class="when" style="display:block">${esc(sub)}</span><span class="when ${state === "configured" ? "ok" : state === "found" || state === "waiting" ? "" : "no"}">${t("st_" + state)}</span></div>`;
  const rows = !found ? "" : [...found.ship.map((n) => line([n.brand, n.model].filter(Boolean).join(" ") || n.device_type || "SHIP", (n.asset ? n.asset + " · " : "") + n.ski.slice(0, 16) + "…", n.state)),
    ...found.modbus.map((m) => {
      const title = m.identity ? `${t("deviceClaim")} · ${m.identity.manufacturer} ${m.identity.model}` : m.host;
      const row = line(title, (m.asset ? m.asset + " · " : "") + m.host, m.state);
      if (m.state !== "found") return row;
      if (!m.review) return row + `<p class="slot">${t("reviewUnsupported")}</p>`;
      const r = m.review;
      return row + `<details class="slot inverter-review"><summary>${t("reviewPv")}</summary>
        <p>${t("reviewRole")} · SunSpec ${esc(m.identity.inverter_model)}</p>
        <p>${esc(r.address)} · ${t("unitLabel")} ${esc(r.unit)}</p>
        <p>${r.refusal ? esc(r.refusal) : t("reviewOk")}</p>
        <pre>${esc(r.toml)}</pre>${r.proposal ? `<button type="button" data-install="${esc(r.proposal)}">${t("applyPv")}</button>` : ""}<button type="button" data-review-close>${t("reviewCancel")}</button></details>`;
    })].join("") || `<div class="slot"><p>${t("findNone")}</p></div>`;
  return `<h2 class="sec">${t("find")}</h2><div class="group" id="discovery"><form class="slot find" id="find">
    <input id="hosts" placeholder="${t("findHosts")}" aria-label="${t("findHosts")}"><button type="submit">${t("findGo")}</button>
    <label>${t("unitLabel")} <input type="number" id="findUnit" min="0" max="255" value="1" required></label>
    <label>Port <select id="findPort"><option>502</option><option>1502</option></select></label></form>${rows}</div>`;

}

function matterReading(point) {
  if (!point) return "";
  const value = point.watts == null ? "—" : new Intl.NumberFormat(lang, { maximumFractionDigits: 3 }).format(point.watts) + " W";
  const state = stale ? (point.received_at ? "stale" : "unavailable") : point.state;
  const received = point.received_at ? new Date(point.received_at).toLocaleString(lang) : "";
  return `<h2 class="sec">${t("matterTitle")}</h2><div class="group"><div class="slot"><b>${esc(point.name)}</b><p>${t("matterPoint")} ${esc(point.endpoint)} · ${t("matterNode")} 0x${esc(point.node)}</p><p>${t("matter_" + state)} · ${value}</p>${received ? `<p>${t("matterTime")} · ${esc(received)}</p>` : ""}</div></div>`;
}
function supplyCard() {
  if (!supplyStatus) return "";
  const {active, next, next_at, expired, price_ct, estimated, can_import} = supplyStatus;
  const review = supplyDraft?.review, terms = review?.terms;
  const when = (seconds) => new Date(seconds * 1000).toLocaleString(locale(), {timeZone: "Europe/Berlin"});
  return `<section id="supplyCard"><h2 class="sec">${t("supply")}</h2><div class="group"><div class="slot"><b>${active ? esc(active.contract) : t("supplyLocal")}</b>
  ${expired ? `<p>${t("supplyExpired")} · ${esc(expired.contract)} (${esc(expired.priced_for_year)})</p>` : ""}
  ${price_ct == null ? "" : `<p>${t(estimated ? "supplyEstimate" : "supplyCurrent")} · ${num(price_ct, 2)} ct/kWh</p>`}${next ? `<p>${t("supplyNext")} · ${esc(next.contract)} · ${esc(when(next_at))}</p>` : ""}</div>
  ${!can_import ? "" : review ? `<div class="slot"><b>${esc(terms.contract)}</b><p>${t("supplyFrom")} · ${esc(when(review.effective_at))}</p><p>${t("supplyPeriod")} · ${esc(terms.priced_for_year)}</p>${terms.valid_until ? `<p>${t("supplyEnd")} · ${esc(terms.valid_until)}</p>` : ""}<p>${review.price_ct == null ? t("supplyNoPrice") : num(review.price_ct, 2) + " ct/kWh"}</p>
  <p>${t("supplySign")} · <bdi>${esc(terms.key_id)}</bdi></p>${(next ?? active ?? expired) && (next ?? active ?? expired).fingerprint !== terms.fingerprint ? `<p><strong>${t("supplyNewKey")}</strong></p>` : ""}<code style="overflow-wrap:anywhere">${esc(terms.fingerprint)}</code><p>${t("supplyCompare")}</p><div class="peerbtns"><button type="button" id="supplyCancel">${t("supplyCancel")}</button><button type="button" class="yes" id="supplyUse">${t("supplyUse")}</button></div></div>` : `<div class="slot"><label>${t("supplyFile")}<input type="file" id="supplyFile" accept=".jws,.jose,application/jose"></label></div>`}</div><p role="status" aria-live="polite">${esc(supplyMessage)}</p></section>`;
}

function wireSupply() {
  const input = $("#supplyFile");
  if (input) input.onchange = async () => {
    const file = input.files[0];
    if (!file || supplyBusy) return;
    supplyMessage = ""; supplyBusy = true; input.disabled = true;
    try {
      if (file.size > 90 * 1024) throw new Error(t("supplyLarge"));
      const signed = await file.text();
      const review = await api("/v1/tariff/preview", {method: "POST", body: JSON.stringify({signed})});
      supplyDraft = {signed, review};
    } catch (e) { supplyMessage = e.message; }
    finally { supplyBusy = false; render(true); $("#supplyCancel")?.focus(); }
  };
  const cancel = $("#supplyCancel"), use = $("#supplyUse");
  if (cancel) cancel.onclick = () => { supplyDraft = null; supplyMessage = ""; render(true); $("#supplyFile")?.focus(); };
  if (use) use.onclick = async () => {
    if (!supplyDraft || supplyBusy) return;
    supplyBusy = true; use.disabled = true; cancel.disabled = true;
    try {
      await api("/v1/tariff", {method: "PUT", body: JSON.stringify({signed: supplyDraft.signed, confirmation: supplyDraft.review})});
      supplyMessage = t("supplySaved");
    } catch (e) { supplyMessage = e.status ? e.message : t("supplyUnknown"); }
    finally {
      supplyDraft = null;
      supplyStatus = await api("/v1/tariff").catch(() => supplyStatus);
      supplyBusy = false; render(true); $("#supplyFile")?.focus();
    }
  };
}

function deviceHealth(h) {
  const devices = trust?.devices;
  if (h.demo || !devices?.length) return "";
  const issues = devices.filter((d) => d.state !== "responding");
  return `<h2 class="sec">${t("healthTitle")}</h2><div class="group"><div class="slot"><b>${t("healthCount", { n: devices.length - issues.length, total: devices.length })}</b></div>
  ${issues.length && trust.support_contact ? `<div class="slot">${esc(trust.support_contact)}</div>` : ""}
  ${issues.map((d) => `<div class="slot"><b>${esc(d.asset)}</b><p>${d.state === "waiting" ? t("healthUnknown") : t("notice_" + d.state)} · ${t(d.state === "disobedient" ? "healthResponse" : "healthCheck")}</p></div>`).join("")}</div>`;
}
function viewSetup(h) {
  const historyIssue = !h.demo && ["repair_pending", "busy", "unavailable"].includes(trust?.history);
  const groups = (s) => (s || "").replace(/[^0-9a-fA-F]/g, "").match(/.{1,4}/g)?.map((g) => `<span>${g}</span>`).join(" ") ?? "–";
  const p = pairing ?? { trusted: [], waiting: [] };
  return `${installationOutcome && !h.demo ? `<div class="slot" role="status">${t((installationOutcome.role === "connection-meter" && ["restarting","applied","restored"].includes(installationOutcome.outcome) ? "meter_" : "change_") + installationOutcome.outcome)}</div>` : ""}${deviceHealth(h)}
  ${historyIssue ? `<h2 class="sec">${t("historyRepairTitle")}</h2><div class="group"><div class="slot" role="status" data-history-status>${t("history_" + trust.history)}</div></div>` : ""}
  ${h.shadow && transfer ? `<h2 class="sec">${t("shadowMode")}</h2><div class="group"><div class="slot"><p>${t("transferHelp", {who: esc(transfer.controller)})}</p>${transfer.readiness !== "ready" ? `<p>${t(transfer.readiness === "meter_missing" ? "transferMeter" : "transferActuator")}</p>` : ""}<button type="button" class="step" ${transfer.readiness !== "ready" && !(transfer.approved_until > Date.now()/1000) ? "disabled" : ""} id="control-transfer" data-transfer-approve="${!(transfer.approved_until > Date.now()/1000)}">${t(transfer.approved_until > Date.now()/1000 ? "revokeTransfer" : "authorizeTransfer")}</button></div></div>` : ""}
  <h2 class="sec">${t("steuerbox")}</h2><div class="group">${trustLines()}
  ${p.waiting.map((w) => `<div class="slot"><b>${t("waiting")}</b><div class="ski" style="font-size:16px;margin-top:6px">${groups(w.ski)}</div>
    <div class="peerbtns"><button data-refuse="${esc(w.ski)}" type="button">${t("refuse")}</button><button class="yes" data-approve="${esc(w.ski)}" type="button">${t("approve")}</button></div></div>`).join("")}
  ${p.trusted.map((s) => `<div class="slot"><span class="when ok">✓ ${t("trusted")}</span><div class="ski" style="font-size:16px">${groups(s)}</div></div>`).join("")}
  ${!p.trusted.length && !p.waiting.length ? `<div class="slot"><p>${t("noPeer")}</p></div>` : ""}</div>
  <details id="operatorIdentity"><summary>${t("forOperator")}</summary><div class="group">
  <div class="slot"><span class="when">${t("ski")}</span><div class="ski">${groups(h.ski)}</div></div>
  ${h.fingerprint ? `<div class="slot" style="border-top:.5px solid var(--sep)"><span class="when">${t("fp")}</span><div class="ski" style="font-size:14px">${groups(h.fingerprint)}</div><p>${t("fpNote")}</p></div>` : ""}</div></details>
  ${h.demo ? "" : supplyCard()}
  ${matterReading(h.matter)}
  ${h.demo ? "" : discovery()}
  ${notifications()}
  ${h.demo ? "" : otherSystems()}
  <h2 class="sec">${t("language")}</h2><div class="group"><div class="row" style="grid-template-columns:1fr auto"><span>${lang === "de" ? "Deutsch" : "English"}</span><button class="link" id="lang" type="button" style="padding:0;width:auto">${lang === "de" ? "English" : "Deutsch"}</button></div></div>
  <h2 class="sec">${t("glance")}</h2><div class="group"><a class="link" href="#glance">${t("open")} ›</a></div><p class="cap" style="margin:8px 16px 0">${t("glanceHelp")}</p>
  ${h.demo ? "" : `<h2 class="sec"></h2><div class="group"><a class="link" href="#handover">${t("sheet")} ›</a><a class="link" href="#record">${t("recordOpen")} ›</a><button class="link" id="newCode" type="button">${t("newCode")}</button></div><p class="cap" style="margin:8px 16px 0">${t("newCodeHelp")}</p><p style="margin-top:10px"><button class="link danger" id="signout" type="button">${t("signout")}</button></p>`}`;
}
function viewSignin() {
  return `<form class="signin" id="signin"><p>${t("signinHelp")}</p>
  <label class="sec" for="tok" style="display:block;margin-left:16px">${t("tokenLabel")}</label><input id="tok" autocomplete="one-time-code" spellcheck="false" required>
  <button class="primary" type="submit">${t("signin")}</button><p class="err" id="err" hidden>${t("signinWrong")}</p></form>`;
}

// ── Render and wire ─────────────────────────────────────────────────────────
function render(refreshDiscovery = false) {
  document.documentElement.lang = lang;
  $(".tabs").setAttribute("aria-label", t("viewsLabel"));
  $("#scrub").setAttribute("aria-label", t("replayTime"));
  $("#play").setAttribute("aria-label", t(playing ? "replayPause" : "replayPlay"));
  document.querySelectorAll("[data-t]").forEach((el) => (el.textContent = t(el.dataset.t)));
  document.querySelectorAll(".tabs a").forEach((a) => a.setAttribute("aria-current", a.dataset.v === view ? "page" : "false"));
  const s = $("#screen"), wall = view === "glance" && home;
  const hour = wall && home.demo && home.at ? Number(new Intl.DateTimeFormat("en-GB", { timeZone: "Europe/Berlin", hour: "2-digit", hourCycle: "h23" }).format(new Date(home.at))) : new Date().getHours();
  const night = wall && (hour >= 22 || hour < 6 || matchMedia("(prefers-color-scheme: dark)").matches);
  document.body.classList.toggle("paper", view === "handover");
  document.documentElement.className = wall ? `glance${night ? " night" : ""}${home.reduction ? " reduced" : ""}` : "";
  awake(!!wall);
  $(".dock").hidden = view === "signin";
  if (view === "signin") { $("#timebar").hidden = true; $("#when").textContent = ""; $("#demo").hidden = true; $("#site").textContent = t("signin"); s.innerHTML = viewSignin(); wire(); return; }
  if (!home) return;
  $("#site").textContent = [home.site || t("home"), home.shadow && t("shadowMode")].filter(Boolean).join(" · ");
  $("#live").className = "live" + (stale ? " off" : "");
  const asBox = !home.demo || capture;
  $("#when").textContent = stale ? t("stale", { t: lastOk ? lastOk.toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit" }) : "–" }) : `${asBox ? t("live") + " · " : ""}${home.at ? hhmm(home.at) : ""}`;
  const d = $("#demo"); d.hidden = asBox; d.textContent = t("demo");
  $("#timebar").hidden = asBox;
  if (home.demo) { $("#scrub").value = at ?? 0; $("#clock").textContent = home.at ? hhmm(home.at) : "–"; $("#playicon").setAttribute("d", playing ? "M7 5h4v14H7zM13 5h4v14h-4z" : "M8 5v14l11-7z"); }
  if (view === "plan" && s.querySelector("[data-editing]")) return;
  s.classList.toggle("demo", !asBox);
  s.classList.toggle("stale", wall ? missedPolls >= 3 : stale);
  if (view === "setup" && !refreshDiscovery && (supplyBusy || document.activeElement?.closest("#supplyCard") || (finding && s.querySelector("#find")) || document.activeElement?.closest("#discovery") || s.querySelector(".inverter-review[open]"))) return;
  if (view === "day" && !refreshDiscovery && (s.querySelector("#gridHistory[open]") || document.activeElement?.closest("#export"))) return;
  const identityOpen = document.querySelector("#operatorIdentity")?.open;
  s.innerHTML = ({ now: viewNow, plan: viewPlan, day: viewDay, setup: viewSetup, glance: viewGlance, handover: viewHandover, record: viewRecord }[view] ?? viewNow)(home);
  if (identityOpen && view === "setup") $("#operatorIdentity").open = true;
  wire();
}
function wire() {
  wireSupply();
  const f = $("#signin");
  if (f) f.onsubmit = async (e) => { e.preventDefault(); if (await signIn($("#tok").value.trim())) { view = "now"; load(); } else $("#err").hidden = false; };
  const st = $("#strip");
  if (st) { const pick = (e) => { const r = st.getBoundingClientRect(); focus = Math.max(0, Math.min(home.plan.length - 1, Math.floor(((e.clientX - r.left) / r.width) * home.plan.length))); render(); }; st.onpointerdown = pick; st.onpointermove = (e) => e.buttons && pick(e); }
  document.querySelectorAll("[data-goal]").forEach((card) => {
    const path = `/v1/goals/${encodeURIComponent(card.dataset.goal)}`, range = card.querySelector("input[type=range]"), by = card.querySelector("select");
    const key = card.dataset.tank ? "target_c" : "target_soc";
    const box = card.querySelector("[data-quote]"), cancel = card.querySelector("[data-cancel]");
    const amount = card.querySelector("[data-amount]"), cap = card.querySelector("[data-cap]");
    const capValue = () => cap?.value ? cap.valueAsNumber : null;
    let revision = 0;
    const draft = () => ({ [key]: card.dataset.tank ? +range.value : +range.value / 100, by_minute: +by.value, ...(capValue() == null ? {} : {price_cap_ct: capValue()}) });
    const current = (id) => card.isConnected && revision === id;
    function message(text) {
      box.hidden = false;
      const status = document.createElement("span");
      status.setAttribute("role", "status");
      status.textContent = text;
      box.replaceChildren(status);
    }
    function action(label, run) {
      const button = document.createElement("button");
      button.type = "button";
      button.className = "primary small";
      button.textContent = label;
      button.onclick = run;
      box.append(button);
    }
    function lock(on) {
      range.disabled = amount.disabled = by.disabled = cancel.disabled = on;
      if (cap) cap.disabled = on;
    }
    function finish() {
      delete card.dataset.editing;
      render();
      document.querySelector(`[data-goal="${CSS.escape(card.dataset.goal)}"] input`)?.focus();
    }
    function begin() {
      revision++;
      card.dataset.editing = "true";
      for (const other of document.querySelectorAll("[data-goal]")) other.inert = other !== card;
      cancel.hidden = false;
      box.hidden = true;
      box.replaceChildren();

      const boost = card.querySelector("[data-boost]");
      if (boost) boost.hidden = true;
      const clear = card.querySelector("[data-clear]");
      if (clear) clear.hidden = true;
    }
    async function quote(body) {
      try { return await api(path + "/quote", { method: "POST", body }); }
      catch (error) {
        if (error.status === 503 || (home.demo && error.status === 404)) return null;
        if (error.status === 429) error.message = t("qBusy");
        throw error;
      }
    }
    function priceText(q) {
      if (q?.extra_eur == null) return t("qNoCost");
      if (Math.abs(q.extra_eur) < 0.005) return t("qFree");
      return t(q.extra_eur > 0 ? "qMore" : "qLess", {eur:eur(Math.abs(q.extra_eur))});
    }
    function display(q) {
      if (!q) return t("qNoCost");
      const price = priceText(q);
      let verdict = t("v_" + q.verdict);
      if (q.verdict === "impossible" && q.reachable != null) {
        const reachable = card.dataset.tank ? num(q.reachable, 1) : Math.floor(q.reachable * 100);
        verdict = t("vt_impossible", { reach: reachable + " " + card.dataset.unit });
      }
      const window = q.eligible_quarters == null ? "" : " · " + t("qEligible", {n:q.eligible_quarters, cap:capValue()});
      return verdict + window + " · " + price;
    }
    async function reconcile(body) {
      message(t("qChecking"));
      try {
        const state = await api("/v1/home");
        if (!card.isConnected) return;
        home = state;
        const saved = (card.dataset.tank ? state.tank_goal : state.goal)?.[0];
        const target = JSON.parse(body);
        if (saved?.[key] === target[key] && saved.by_minute === target.by_minute && (saved.price_cap_ct ?? null) === (target.price_cap_ct ?? null)) { finish(); return; }
        card.querySelector(".cap").textContent = saved ? t("qSaved") + " · " +
          (card.dataset.tank ? saved[key] : Math.round(saved[key] * 100)) + " " + card.dataset.unit + " · " + minToHHMM(saved.by_minute) + capLabel(saved.price_cap_ct) : t("qNone");
        lock(false);
        await ask();
      } catch (error) {
        if (!card.isConnected) return;
        message(t("qChecking") + " " + error.message);
        action(t("qChecking"), () => reconcile(body));
        window.addEventListener("online", () => {
          if (card.isConnected && range.disabled) reconcile(body);
        }, { once: true });
      }
    }
    async function ask() {
      begin();
      if (!amount.reportValidity() || (cap && !cap.reportValidity())) return;
      const id = revision, body = JSON.stringify(draft());
      message(t("qAsk"));
      try {
        const first = await quote(body);
        if (!current(id)) return;
        function offer(q, changed) {
          const checkedAt = performance.now();
          message((changed ? t("qChanged") + " " : "") + display(q));
          const label = t("qSet") + " · " + range.value + " " + card.dataset.unit + " · " + minToHHMM(+by.value) + capLabel(capValue());
          action(label, async () => {
            lock(true);
            message(t("qAsk"));
            try {
              // The box admits one solve per second. Recheck the visible price before writing.
              const wait = 1050 - (performance.now() - checkedAt);
              if (q && wait > 0) await new Promise(resolve => setTimeout(resolve, wait));
              if (!current(id)) return;
              const latest = await quote(body);
              if (!current(id)) return;
              if (display(latest) !== display(q)) { lock(false); offer(latest, true); return; }
              await api(path, { method: "PUT", body });
              if (current(id)) await reconcile(body);
            } catch (error) {
              if (!current(id)) return;
              if (error.status) { lock(false); message(error.message); action(t("qRetry"), ask); }
              else await reconcile(body);
            }
          });
        }
        offer(first, false);
      } catch (error) {
        if (current(id)) { message(error.message); action(t("qRetry"), ask); }
      }
    }
    const boost = card.querySelector("[data-boost]");
    if (boost) boost.onclick = async () => {
      begin();
      const id = revision, goal = home.goal[0];
      async function checkState() {
        const state = await api("/v1/home");
        if (current(id)) { home = state; lock(false); finish(); }
      }
      try {
        if (home.charging_now) {
          await api(path + "/boost", {method:"DELETE"});
          await checkState();
          return;
        }
        message(t("qAsk"));
        const q = await api(path + "/quote?charge_now=true", {method:"POST", body:JSON.stringify(goal)});
        if (!current(id)) return;
        const pricedAt = performance.now();
        message(t("qAbout", {price:priceText(q)}) + " · " + Math.round(goal.target_soc * 100) + " % · " + minToHHMM(goal.by_minute) + capLabel(goal.price_cap_ct));
        action(t("qBoost"), async () => {
          lock(true);
          message(t("qAsk"));
          const wait = 1050 - (performance.now() - pricedAt);
          if (wait > 0) await new Promise(resolve => setTimeout(resolve, wait));
          if (!current(id)) return;
          try {
            await api(path + "/boost", {method:"PUT", body:JSON.stringify({goal, extra_eur:q.extra_eur, until:q.boost_until})});
            await checkState();
          } catch (error) {
            if (!current(id)) return;
            lock(false);
            message(error.message);
            action(t("qBoostCheck"), () => checkState().catch(e => message(e.message)));
          }
        });
      } catch (error) {
        if (current(id)) {message(error.message); action(t("qBoostCheck"), () => checkState().catch(e => message(e.message)));}
      }
    };
    range.oninput = () => {
      amount.value = range.value;
      begin();
      const saved = (card.dataset.tank ? home.tank_goal : home.goal)?.[0];
      const rate = home.devices.find(d => d.id === card.dataset.goal)?.goal_eur_per_unit;
      if (home.previous_plan_at || !Number.isFinite(rate) || !saved || saved.by_minute !== +by.value || (saved.price_cap_ct ?? null) !== capValue()) return;
      const baseline = card.dataset.tank ? saved.target_c : saved.target_soc * 100;
      const difference = (+range.value - baseline) * rate;
      box.hidden = false;
      box.textContent = t("qAbout", {price: t(difference < 0 ? "qLess" : "qMore", {eur: eur(Math.abs(difference))})});
    };
    range.onchange = () => { amount.value = range.value; return ask(); };
    amount.oninput = () => { begin(); if (amount.validity.valid) range.value = amount.value; };
    amount.onchange = () => { if (amount.validity.valid) range.value = amount.value; return ask(); };
    if (cap) { cap.oninput = begin; cap.onchange = ask; }
    by.onchange = ask;
    cancel.onclick = finish;
    const clear = card.querySelector("[data-clear]");
    if (clear) clear.onclick = async () => {
      try { await api(path, { method: "DELETE" }); await load(); } catch (error) { toast(error.message); }
    };
  });
  document.querySelectorAll("[data-approve]").forEach((b) => (b.onclick = async () => { pairing = await api("/v1/pairing", { method: "POST", body: JSON.stringify({ ski: b.dataset.approve }) }); toast(t("approved")); render(); }));
  document.querySelectorAll("[data-refuse]").forEach((b) => (b.onclick = async () => { pairing = await api(`/v1/pairing/${encodeURIComponent(b.dataset.refuse)}/refuse`, { method: "POST" }); toast(t("refused")); render(); }));
  const lg = $("#lang"); if (lg) lg.onclick = () => { lang = lang === "de" ? "en" : "de"; try { localStorage.setItem("hems.lang", lang); } catch {} render(); if (mine()) savePush({ lang }).catch(() => {}); };
  const po = $("#pushOn"); if (po) po.onclick = pushOn;
  const pf = $("#pushOff"); if (pf) pf.onclick = pushOff;
  const pt = $("#pushTest"); if (pt) pt.onclick = async () => { try { await api("/v1/push/test", { method: "POST", body: JSON.stringify({ endpoint: sub.endpoint }) }); toast(t("pushSent")); } catch (e) { toast(e.message); loadPush(); } };
  document.querySelectorAll("[data-push]").forEach((b) => (b.onchange = () => {
    const m = mine(), id = b.dataset.push;
    savePush(id === "quiet" ? { quiet: b.checked ? NIGHT : null } : { events: EVENTS.filter((e) => (e === id ? b.checked : m.events.includes(e))) }).catch((e) => toast(e.message));
  }));
  const ha = $("#hookAdd"); if (ha) ha.onsubmit = async (e) => {
    e.preventDefault();
    try {
      const made = await api("/v1/subscriptions", { method: "POST", body: JSON.stringify({ name: $("#hookName").value.trim(), url: $("#hookUrl").value.trim(), types: [$("#hookWhat").value], public: $("#hookPublic").checked }) });
      hookSecret = made.secret; await loadHooks();
    } catch (err) { toast(err.message); }
  };
  const hc = $("#hookCopy"); if (hc) hc.onclick = async () => { try { await navigator.clipboard.writeText(hookSecret); toast(t("hookCopied")); } catch { /* the code is selectable */ } };
  document.querySelectorAll("[data-hook-test]").forEach((b) => (b.onclick = async () => {
    try { const r = await api(`/v1/subscriptions/${encodeURIComponent(b.dataset.hookTest)}/test`, { method: "POST" }); toast(r.delivered ? t("hookOk", { ms: r.millis }) : r.error); } catch (e) { toast(e.message); }
  }));
  document.querySelectorAll("[data-hook-remove]").forEach((b) => (b.onclick = async () => {
    try { await api(`/v1/subscriptions/${encodeURIComponent(b.dataset.hookRemove)}/revoke`, { method: "POST" }); hookSecret = null; await loadHooks(); } catch (e) { toast(e.message); }
  }));
  const tx = $("#control-transfer");
  if (tx) tx.onclick = async () => {
    const approve = tx.dataset.transferApprove === "true";
    if (approve && !confirm(t("transferConfirm"))) return;
    tx.disabled = true;
    try { transfer = await api("/v1/commissioning/transfer", {method:"PUT", body:JSON.stringify({id:transfer.id, approve})}); render(true); }
    catch (error) { toast(error.message); tx.disabled = false; }
  };
  const pr = $("#print"); if (pr) pr.onclick = () => window.print();
  const so = $("#signout"); if (so) so.onclick = async () => { try { await api("/v1/session", { method: "DELETE" }); location.reload(); } catch (e) { if (view !== "signin") toast(e.message); } };
  const ag = $("#again"); if (ag) ag.onsubmit = (e) => { e.preventDefault(); code = $("#tok2").value.trim(); loadSigninQr().then(() => { if (!signinQr) { code = null; toast(t("signinWrong")); } }); };
  const od = $("#order"); if (od) od.onchange = () => { order = od.value.trim().replace(/[^\x21-\x7e]/g, ""); render(); };
  const rq = $("#recordQr"); if (rq) rq.onerror = () => { rq.hidden = true; $("#recordBig").hidden = false; };
  const fd = $("#find");
  if (fd) fd.onsubmit = async (event) => {
    event.preventDefault();
    const hosts = $("#hosts").value, unit = $("#findUnit").value, port = $("#findPort").value;
    if (finding) return;
    finding = true;
    const button = fd.querySelector("button"); button.disabled = true;
    try {
      const result = await api("/v1/discover", { method: "POST", body: JSON.stringify({ hosts: hosts.split(/[\s,;]+/).filter(Boolean), unit: Number(unit), port: Number(port) }) });
      if (!fd.isConnected) return;
      found = result; render(true);
      $("#hosts").value = hosts; $("#findUnit").value = unit; $("#findPort").value = port;
      ($(".inverter-review summary") ?? $("#find button")).focus();
    } catch (err) { toast(err.message); }
    finally { finding = false; button.disabled = false; }
  };
  document.querySelectorAll("[data-install]").forEach((button) => {
    button.onclick = async () => {
      if (!confirm(t("applyConfirm"))) return;
      button.disabled = true;
      const id = button.dataset.install;
      try {
        installationOutcome = await api("/v1/commissioning/change", {method:"POST", body:JSON.stringify({id})});
        found = null;
      } catch (error) {
        if (error.status) toast(error.message);
        else installationOutcome = {id, outcome:"unknown"};
      }
      render(true);
    };
  });
  document.querySelectorAll("[data-review-close]").forEach((button) => {
    button.onclick = () => {
      const panel = button.closest("details"); panel.open = false; panel.querySelector("summary").focus();
    };
  });
  const nc = $("#newCode"); if (nc) nc.onclick = async () => {
    const old = code ?? prompt(t("tokenLabel")); if (!old || !confirm(t("newCodeHelp"))) return;
    const r = await fetch("/v1/token", { method: "POST", headers: { authorization: `Bearer ${old}`, "content-type": "application/json" } });
    if (!r.ok) return toast(t("signinWrong"));
    signinQr = null; if (await signIn((await r.json()).token)) { toast(t("newCodeDone")); location.hash = "handover"; }
  };
  const history = $("#gridHistory");
  if (history) history.ontoggle = () => {
    if (history.open) inspectHistory(history);
  };
  const ex = $("#export");
  if (ex) { ex.onclick = exportAll; showExportStatus(); }
  document.querySelectorAll("[data-day]").forEach((b) => {
    b.onclick = () => { picked = picked === b.dataset.day ? null : b.dataset.day; render(true); };
    b.onkeydown = (event) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      const day = b.dataset.day;
      b.onclick();
      document.querySelector(`[data-day="${CSS.escape(day)}"]`)?.focus();
    };
  });
  document.querySelectorAll("[data-month]").forEach((b) => (b.onclick = () => { monthKey = b.dataset.month; picked = null; render(true); }));
  const lt = $("#latest"); if (lt) lt.onclick = () => { picked = null; render(true); window.scrollTo(0, 0); };
}
const quarterMs = 15 * 60_000;
const berlinDate = new Intl.DateTimeFormat("sv-SE", {timeZone: "Europe/Berlin"});

function inspectHistory(panel) {
  const lastClosed = Math.floor(Date.now() / quarterMs) * quarterMs - quarterMs;
  const latestDay = berlinDate.format(lastClosed);
  const earliestDay = berlinDate.format(lastClosed - 3 * 365 * 86_400_000);
  const date = picked && picked >= earliestDay && picked <= latestDay ? picked : latestDay;
  panel.querySelector(".history-body").innerHTML = `<form class="history-controls"><label>${t("historyDate")}<input type="date" min="${earliestDay}" max="${latestDay}" value="${date}" required></label>
    <label>${t("historyQuarter")}<select></select></label></form>
    <div class="history-result" role="status"></div>`;
  const form = panel.querySelector("form"), day = form.querySelector("input"), quarter = form.querySelector("select");
  const result = panel.querySelector(".history-result");
  const labels = new Intl.DateTimeFormat(locale(), {hour: "2-digit", minute: "2-digit", timeZoneName: "shortOffset", timeZone: "Europe/Berlin"});
  let rows, request = 0;
  function showQuarter() {
    const r = rows.get(Number(quarter.value));
    if (!r) { result.textContent = t("historyEmpty"); return; }
    const power = watts => Number.isFinite(watts) ? `${watts.toLocaleString(locale(), {maximumFractionDigits: 3})} W` : t("historyUnknown");
    const values = [["historyAverage", power(r.watts)], ["historyMin", power(r.min_watts)], ["historyMax", power(r.max_watts)], ["historySamples", Number.isInteger(r.samples) ? num(r.samples, 0) : t("historyUnknown")]];
    result.innerHTML = `<dl>${values.map(([key, value]) => `<div><dt>${t(key)}</dt><dd>${value}</dd></div>`).join("")}</dl><p class="cap">${t("historySign")}</p>`;
  }
  async function loadDay() {
    const revision = ++request;
    quarter.replaceChildren();
    quarter.disabled = true;
    result.replaceChildren();
    if (!day.value || !day.validity.valid) return;
    // Enumerate real instants so a Berlin clock change offers 92 or 100 quarters.
    const midnight = Date.parse(day.value + "T00:00:00Z"), slots = [];
    for (let at = midnight - 2 * 3_600_000; at < midnight + 24 * 3_600_000 && at <= lastClosed; at += quarterMs) {
      if (berlinDate.format(at) === day.value) slots.push(at);
    }
    if (!slots.length) return;
    for (const at of slots) quarter.add(new Option(labels.format(at), String(at)));
    quarter.value = String(slots.at(-1));
    const q = new URLSearchParams({resolution: "quarter_hours", from: new Date(slots[0]).toISOString(), to: new Date(slots.at(-1) + quarterMs).toISOString()});
    result.textContent = t("historyLoading");
    try {
      const window = await api("/v1/series/grid?" + q);
      if (revision !== request || !panel.isConnected) return;
      rows = new Map(window.readings.map(r => [Date.parse(r.at), r]));
      quarter.disabled = false;
      showQuarter();
    } catch (err) {
      if (revision !== request || !panel.isConnected) return;
      result.textContent = t(err.status === 404 ? "historyOff" : [429, 503].includes(err.status) ? "historyBusy" : "historyFailure");
      if (err.status !== 404) {
        const retry = document.createElement("button");
        retry.type = "button"; retry.className = "link"; retry.textContent = t("historyRetry");
        retry.onclick = () => { day.focus(); loadDay(); };
        result.append(retry);
      }
    }
  }
  form.onsubmit = event => event.preventDefault();
  day.onchange = loadDay;
  quarter.onchange = showQuarter;
  loadDay();
}

let exportStatus = null;
function showExportStatus() {
  const button = $("#export");
  if (!button) return;
  button.disabled = exportStatus?.key === "exportBusy";
  $("#exportStatus").textContent = exportStatus ? t(exportStatus.key, {points: exportStatus.points?.join(", ")}) : "";
}
async function exportAll() {
  if (!home || exportStatus?.key === "exportBusy") return;
  const site = home.site, points = [...new Set(["grid", "netzwirksam", ...home.devices.map(d => d.id)])];
  const requestedAt = new Date(), end = Math.ceil(requestedAt.getTime() / quarterMs) * quarterMs;
  const from = new Date(end - 3 * 365 * 86_400_000).toISOString(), to = new Date(end).toISOString();
  const q = new URLSearchParams({from, to, resolution: "quarter_hours"});
  const chunks = [], failures = [];
  exportStatus = {key: "exportBusy"}; showExportStatus();
  try {
    for (const point of points) {
      try {
        const series = await api(`/v1/series/${encodeURIComponent(point)}?${q}`);
        chunks.push(new Blob([chunks.length ? "," : "", JSON.stringify(point), ":", JSON.stringify(series)]));
      } catch (error) {
        if (!home || home.site !== site) return;
        failures.push({point, status: error.status ?? null});
      }
    }
    if (chunks.length) {
      const meta = {site, requested_at: requestedAt.toISOString(), exported_at: new Date().toISOString(), from, to, complete: failures.length === 0, failures};
      const blob = new Blob([JSON.stringify(meta).slice(0, -1), ',"series":{', ...chunks, "}}"], {type: "application/json"});
      const url = URL.createObjectURL(blob), a = document.createElement("a");
      try {
        a.href = url; a.download = `hems-${site}-${to.slice(0, 10)}${failures.length ? "-partial" : ""}.json`; a.click();
      } finally { setTimeout(() => URL.revokeObjectURL(url), 1000); }
    }
    let key = failures.length ? "exportPartial" : "exportDone";
    if (!chunks.length) key = failures.every(f => f.status === 404) ? "historyOff" : "exportFailed";
    exportStatus = {key, points: failures.map(f => f.point)};
  } catch {
    exportStatus = {key: "exportFailed", points};
  } finally {
    if (exportStatus?.key === "exportBusy") exportStatus = null;
    showExportStatus();
  }
}

// ── Start ───────────────────────────────────────────────────────────────────
function takeOrder() {
  const rec = location.hash.match(/^#record=([\x21-\x7e]{1,64})$/);
  if (!rec) return;
  try { order = decodeURIComponent(rec[1]).replace(/[^\x21-\x7e]/g, "").slice(0, 64); } catch { order = ""; }
  history.replaceState(null, "", location.pathname + location.search + "#record");
}
async function start() {
  // A sign-in link from the handover sheet carries the code in the fragment,
  // which never reaches a server log; exchange it once and wipe it.
  const m = location.hash.match(/^#t=([\w-]+)$/);
  if (m) { await signIn(m[1]); history.replaceState(null, "", location.pathname + "#now"); }
  // An order system's link can carry the order number, so nobody types a UUID
  // at the wall: <box>/#record=<reference>.
  takeOrder();
  view = VIEWS.includes(location.hash.slice(1)) ? location.hash.slice(1) : "now";
  window.addEventListener("hashchange", () => { takeOrder(); const v = location.hash.slice(1); if (view !== "signin" && VIEWS.includes(v)) { view = v; if (v === "setup") { loadPairing(); loadPush(); loadTrust(); loadHooks(); } else { hookSecret = null; } if (v === "handover") loadSheet(); if (v === "day" || v === "plan") loadDays(); render(); window.scrollTo(0, 0); } });
  $("#scrub").addEventListener("input", (e) => { at = +e.target.value; playing = false; load(); });
  $("#play").addEventListener("click", () => { playing = !playing; render(); });
  await load();
  if (view === "setup") { loadPush(); loadHooks(); }
  if (view === "handover") loadSheet();
  if (view === "day" || view === "plan") loadDays();
  setInterval(() => { if (home?.demo) { if (playing) { at = ((at ?? 0) + 5) % 1440; load(); } } else if (view !== "signin") load(); }, home?.demo ? 350 : 5000);
}
// The capture hook: move the replay to `minute` of the day and render it.
if (capture) window.hemsAt = async (minute) => { at = minute; playing = false; await load(); };
async function loadDays() { try { days = (await api("/v1/days")).days; } catch { days = null; } render(); }
async function loadPairing() { try { pairing = await api("/v1/pairing"); } catch { pairing = null; } render(); }
start();
