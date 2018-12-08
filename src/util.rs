//  ff-node-monitor -- Monitoring for Freifunk nodes
//  Copyright (C) 2018  Ralf Jung <post AT ralfj DOT de>
//
//  This program is free software: you can redistribute it and/or modify
//  it under the terms of the GNU Affero General Public License as published by
//  the Free Software Foundation, either version 3 of the License, or
//  (at your option) any later version.
//
//  This program is distributed in the hope that it will be useful,
//  but WITHOUT ANY WARRANTY; without even the implied warranty of
//  MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
//  GNU Affero General Public License for more details.
//
//  You should have received a copy of the GNU Affero General Public License
//  along with this program.  If not, see <https://www.gnu.org/licenses/>.

use rocket::{
    Request,
    Outcome,
    State,
    request::{Outcome as ReqOutcome, FromRequest, FromFormValue},
    response::Responder,
    http::{Status, RawStr},
};
use rocket_contrib::templates::Template;

use failure::{Fail, Error, bail};
use lettre_email;
use lettre::smtp::{SMTP_PORT, SmtpTransport, SmtpClient, ClientSecurity};
use serde_derive::Serialize;

use crate::config::Config;

use std::ops::Deref;

/// Module for serde "with" to use hex encoding to byte arrays
pub mod hex_signing_key {
    use hex;
    use serde::{Deserializer, Deserialize, de::Error};
    use ring::{digest, hmac};

    pub fn deserialize<'de, D>(deserializer: D) -> Result<hmac::SigningKey, D::Error>
        where D: Deserializer<'de>
    {
         let bytes = hex::decode(String::deserialize(deserializer)?).map_err(Error::custom)?;
         Ok(hmac::SigningKey::new(&digest::SHA256, bytes.as_slice()))
    }
}

/// Macro for generating URLs with query parameters
macro_rules! url_query {
    ( $url:expr, $( $name:ident = $value:expr ),* ) => {
        {
            let mut url = $url;
            url.query_pairs_mut()
                $(.append_pair(stringify!($name), $value.as_ref()))*;
            url
        }
    };
}

/// Type for email addresses in Rocket forms
#[derive(Serialize)]
pub struct EmailAddress(String);

impl<'v> FromFormValue<'v> for EmailAddress {
    type Error = Error;

    fn from_form_value(v: &'v RawStr) -> Result<EmailAddress, Error> {
        let s = v.url_decode()?;
        {
            let email_parts : Vec<&str> = s.split('@').collect();
            if email_parts.len() != 2 {
                bail!("Too many or two few @");
            }
            if email_parts[0].is_empty() {
                bail!("User part is empty");
            }
            if email_parts[1].find('.').is_none() {
                bail!("Domain part must contain .");
            }
        }
        Ok(EmailAddress(s))
    }
}

impl Deref for EmailAddress {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// Horribly hacky hack to get access to the Request, and then a template's body, for building emails
pub struct EmailBuilder<'a, 'r: 'a> {
    request: &'a Request<'r>,
    config: &'a Config,
}

#[derive(Debug, Fail)]
enum ResponderError {
    #[fail(display = "responder failed with status {}", status)]
    RenderFailure {
        status: Status,
    },
    #[fail(display = "couldn't find a body")]
    NoBody,
}

impl<'a, 'r> FromRequest<'a, 'r> for EmailBuilder<'a, 'r> {
    type Error = ();
    fn from_request(request: &'a Request<'r>) -> ReqOutcome<Self, Self::Error> {
        let config = request.guard::<State<Config>>()?.inner();
        Outcome::Success(EmailBuilder { request, config })
    }
}

impl<'a, 'r> EmailBuilder<'a, 'r> {
    fn responder_body<'re>(&self, responder: impl Responder<'re>) -> Result<String, Error> {
        let mut resp = responder.respond_to(self.request)
            .map_err(|status| ResponderError::RenderFailure { status })?;
        Ok(resp.body_string().ok_or(ResponderError::NoBody)?)
    }

    /// Begin building an email from a template
    pub fn email(&self, email_template: Template) -> Result<lettre_email::EmailBuilder, Error> {
        let email_text = self.responder_body(email_template)?;
        let email_parts : Vec<&str> = email_text.splitn(4, '\n').collect();
        let (empty, email_from, email_subject, email_body) = (email_parts[0], email_parts[1], email_parts[2], email_parts[3]);
        assert!(empty.is_empty(), "The first line of the email template must be empty");

        // Build email
        Ok(lettre_email::EmailBuilder::new()
            .from((self.config.ui.email_from.as_str(), email_from))
            .subject(email_subject)
            .text(email_body))
    }

    /// Construct a mailer
    pub fn mailer(&self) -> Result<SmtpTransport, Error> {
        let host = self.config.secrets.get_smtp_host();
        Ok(SmtpClient::new((host, SMTP_PORT), ClientSecurity::None)?.transport())
    }
}
