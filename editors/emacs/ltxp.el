;;; ltxp.el --- Instant LaTeX preview with exact two-way SyncTeX -*- lexical-binding: t -*-

;; URL: https://github.com/SichangHe/latex_live_preview
;; Version: 0.0.5
;; Package-Requires: ((emacs "29.1"))
;; SPDX-License-Identifier: UPL-1.0

;;; Commentary:
;; Runs the `ltxp' language server through Eglot.  Nothing starts on its own: `M-x
;; ltxp-preview' in a TeX buffer starts ltxp and opens the preview in the browser.  Then typing
;; updates the preview, clicking the preview moves point to the clicked character, and moving
;; point marks the same spot in the preview.  `M-x ltxp-stop' stops it.  Call `ltxp-setup'
;; from your init file; it only registers the server with Eglot.

;;; Code:
(require 'eglot)

(defgroup ltxp nil "Instant LaTeX preview." :group 'tex)

(defcustom ltxp-command '("ltxp")
  "Command and arguments that start the ltxp language server."
  :type '(repeat string))

(defcustom ltxp-cursor-delay 0.1
  "Seconds after point stops moving before the preview marker follows it."
  :type 'number)

(defconst ltxp-modes '(latex-mode LaTeX-mode tex-mode TeX-mode plain-tex-mode latex-ts-mode)
  "Major modes whose buffers use ltxp.")

(defvar-local ltxp--timer nil)

(defun ltxp--send-cursor (buffer)
  "Tell the server where point is in BUFFER."
  (when (buffer-live-p buffer)
    (with-current-buffer buffer
      (when-let ((server (eglot-current-server)))
        (jsonrpc-notify server :ltxp/cursor
                        (list :textDocument (eglot--TextDocumentIdentifier)
                              :position (eglot--pos-to-lsp-position)))))))

(defun ltxp--post-command ()
  "Send the cursor position once point rests."
  (when ltxp--timer (cancel-timer ltxp--timer))
  (setq ltxp--timer (run-with-timer ltxp-cursor-delay nil #'ltxp--send-cursor (current-buffer))))

;;;###autoload
(define-minor-mode ltxp-mode
  "Send point moves to the ltxp preview.  Turned on by `ltxp-preview'."
  :lighter " ltxp"
  (if ltxp-mode
      (add-hook 'post-command-hook #'ltxp--post-command nil t)
    (remove-hook 'post-command-hook #'ltxp--post-command t)))

(defun ltxp--open (server)
  "Open SERVER's preview in the browser."
  (let ((url (jsonrpc-request server :workspace/executeCommand
                              '(:command "ltxp.previewUrl" :arguments []))))
    (if (stringp url)
        (progn (browse-url url) (message "ltxp: preview at %s" url))
      (user-error "ltxp: no preview: no \\documentclass here, and no main file found"))))

;;;###autoload
(defun ltxp-preview ()
  "Start ltxp for this TeX buffer and open the preview."
  (interactive)
  (unless (apply #'derived-mode-p ltxp-modes)
    (user-error "ltxp: `ltxp-preview' needs a TeX buffer"))
  (unless (executable-find (car ltxp-command))
    (user-error "ltxp: %s not found; install ltxp or set `ltxp-command'" (car ltxp-command)))
  (let ((server (or (eglot-current-server)
                    (let ((eglot-sync-connect t))
                      (apply #'eglot (eglot--guess-contact))
                      (eglot-current-server)))))
    (unless server (user-error "ltxp: could not start %s; see *EGLOT events*" (car ltxp-command)))
    (dolist (b (eglot--managed-buffers server))
      (with-current-buffer b (ltxp-mode 1)))
    (ltxp--open server)))

(defun ltxp-stop ()
  "Stop the ltxp preview of this project."
  (interactive)
  (when-let ((server (eglot-current-server)))
    (dolist (b (eglot--managed-buffers server))
      (with-current-buffer b (ltxp-mode -1)))
    (eglot-shutdown server)))

;;;###autoload
(defun ltxp-setup ()
  "Register ltxp with Eglot for TeX buffers.  Starts nothing; see `ltxp-preview'."
  (add-to-list 'eglot-server-programs
               (cons ltxp-modes (lambda (&rest _) (append ltxp-command '(:initializationOptions (:openBrowser :json-false)))))))

(provide 'ltxp)
;;; ltxp.el ends here
